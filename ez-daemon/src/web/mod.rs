//! Web front door: mounts the `/api/*` REST control surface (`api`), the `/ws/*` WebSocket
//! control and data planes (`ws`), and the compiled frontend's static assets onto one axum
//! router, then serves it on its own listener — concurrently with, and fully independent of,
//! `crate::server`'s TCP control/data path. A stalled or malicious web client can only ever
//! stall this listener's own connection tasks; it shares no lock or queue with the TCP path
//! beyond the same read-mostly [`DaemonState`] both transports are built on.
//!
//! No workflow/pipeline-framework abstraction here or anywhere downstream in this module —
//! `serve` is a single direct call into `axum::serve` over a router built from plain
//! function composition, matching the rest of the crate's style (see the crate-root doc
//! comment).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::state::DaemonState;

pub mod api;
pub mod frontend;
pub mod wire;
pub mod ws;

const ACCEPT_POLL: Duration = Duration::from_millis(100);

/// Serves the web UI/API on `listener` until `running` clears. `static_dir`, when it exists
/// on disk, is served at `/` with a SPA-style fallback to `static_dir/index.html` for any
/// path that isn't a real file (client-side routing); when it doesn't exist (e.g. a
/// from-source `cargo run` with no frontend build yet), the daemon still serves `/api/*` and
/// `/ws/*` — a missing frontend build is a deployment gap, not a reason to fail control-plane
/// availability.
pub async fn serve(
    listener: TcpListener,
    state: Arc<DaemonState>,
    running: Arc<AtomicBool>,
    static_dir: PathBuf,
) -> Result<()> {
    let local_addr = listener.local_addr().ok();
    tracing::info!(?local_addr, ?static_dir, "ez-daemon web server listening");

    let mut router = api::router().merge(ws::router(Arc::clone(&running)));

    if static_dir.is_dir() {
        let index = static_dir.join("index.html");
        let serve_dir = ServeDir::new(&static_dir).fallback(ServeFile::new(index));
        router = router.fallback_service(serve_dir);
    } else {
        tracing::warn!(
            ?static_dir,
            "frontend static directory not found; serving bundled browser console"
        );
        router = router.merge(frontend::router());
    }

    let app = router
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            transport_request_guard,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let server = axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(wait_for_shutdown(running));
    server.await?;
    tracing::info!("ez-daemon web server shutting down");
    Ok(())
}

/// Applies one browser trust policy to both REST requests and WebSocket upgrades.
/// Native clients without an Origin remain supported. Browser loopback origins may
/// reach loopback hosts; remote browser origins must name the same numeric endpoint
/// as the request Host, matching the WebSocket policy's trusted-network behavior.
pub(crate) fn browser_origin_allowed(headers: &HeaderMap) -> bool {
    let Some(request_host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    if !valid_authority(request_host) || !trusted_host(request_host) {
        return false;
    }

    // Native clients do not send Origin and remain supported after Host syntax is checked.
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    if !matches!(uri.scheme_str(), Some("http" | "https")) {
        return false;
    }
    let Some(authority) = uri.authority() else {
        return false;
    };
    if !valid_authority(authority.as_str())
        || uri
            .path_and_query()
            .is_some_and(|path| path.as_str() != "/")
    {
        return false;
    }

    let origin_host = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let request_authority = match request_host.parse::<axum::http::uri::Authority>() {
        Ok(authority) => authority,
        Err(_) => return false,
    };
    let request_host_name = request_authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');

    if origin_host.eq_ignore_ascii_case("localhost") {
        return request_host_name.eq_ignore_ascii_case("localhost")
            || request_host_name.parse::<std::net::IpAddr>().is_ok();
    }
    let Ok(origin_ip) = origin_host.parse::<std::net::IpAddr>() else {
        return false;
    };
    if origin_ip.is_loopback() {
        return request_host_name.eq_ignore_ascii_case("localhost")
            || request_host_name.parse::<std::net::IpAddr>().is_ok();
    }
    !origin_ip.is_unspecified() && request_host.eq_ignore_ascii_case(authority.as_str())
}

fn valid_authority(value: &str) -> bool {
    if value.contains('@') {
        return false;
    }
    let Ok(authority) = value.parse::<axum::http::uri::Authority>() else {
        return false;
    };
    !authority.host().is_empty()
        && (authority.as_str().len() == authority.host().len() || authority.port_u16().is_some())
}

fn trusted_host(value: &str) -> bool {
    let Ok(authority) = value.parse::<axum::http::uri::Authority>() else {
        return false;
    };
    let host = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| !ip.is_unspecified())
}

async fn transport_request_guard(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !browser_origin_allowed(request.headers()) {
        return (StatusCode::FORBIDDEN, "Untrusted Host or browser Origin").into_response();
    }
    if request.uri().path().starts_with("/api/") && state.authentication_required() {
        let valid = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .is_some_and(|(scheme, token)| {
                scheme.eq_ignore_ascii_case("bearer")
                    && !token.is_empty()
                    && !token.contains(' ')
                    && state.authenticate(token)
            });
        if !valid {
            return (StatusCode::UNAUTHORIZED, "Authentication required").into_response();
        }
    }
    next.run(request).await
}

async fn wait_for_shutdown(running: Arc<AtomicBool>) {
    while running.load(Ordering::Relaxed) {
        tokio::time::sleep(ACCEPT_POLL).await;
    }
}
