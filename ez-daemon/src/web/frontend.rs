//! Browser console bundled in the daemon executable.
use crate::state::DaemonState;
use axum::{
    http::{header, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use std::sync::Arc;

pub fn router() -> Router<Arc<DaemonState>> {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("frontend/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [
                        (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                        (header::CACHE_CONTROL, "no-cache"),
                    ],
                    include_str!("frontend/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("frontend/style.css"),
                )
            }),
        )
        .route(
            "/favicon.ico",
            get(|| async { StatusCode::NO_CONTENT.into_response() }),
        )
}
