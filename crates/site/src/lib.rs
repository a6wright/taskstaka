//! taskstaka.com. Pages are rendered from Markdown once, at startup, and
//! served from memory; later this same service grows the sync endpoints.

use axum::Router;
use axum::routing::get;

pub fn router() -> Router {
    Router::new().route("/healthz", get(|| async { "ok" }))
}

/// `$PORT` as Cloud Run sets it; 8080 when unset, empty or not a number.
pub fn port_from_env(var: Option<&str>) -> u16 {
    var.and_then(|v| v.trim().parse().ok()).unwrap_or(8080)
}
