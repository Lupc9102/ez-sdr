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
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::state::DaemonState;

pub mod api;
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

    let mut router = api::router().merge(ws::router());

    if static_dir.is_dir() {
        let index = static_dir.join("index.html");
        let serve_dir = ServeDir::new(&static_dir).fallback(ServeFile::new(index));
        router = router.fallback_service(serve_dir);
    } else {
        tracing::warn!(
            ?static_dir,
            "frontend static directory not found; serving /api and /ws only"
        );
    }

    let app = router
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let server = axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(wait_for_shutdown(running));
    server.await?;
    tracing::info!("ez-daemon web server shutting down");
    Ok(())
}

async fn wait_for_shutdown(running: Arc<AtomicBool>) {
    while running.load(Ordering::Relaxed) {
        tokio::time::sleep(ACCEPT_POLL).await;
    }
}
