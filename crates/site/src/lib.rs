//! taskstaka.com. Pages are rendered from Markdown once, at startup, and
//! served from memory; later this same service grows the sync endpoints.

pub mod diagrams;
pub mod pages;

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;

pub fn router() -> Router {
    let mut r = Router::new().route("/healthz", get(|| async { "ok" }));
    for page in pages::all() {
        let html: Arc<str> = Arc::from(page.html);
        r = r.route(page.path, get(move || page_response(html.clone())));
    }
    r.fallback(fallback)
}

async fn page_response(html: Arc<str>) -> Response {
    (
        [(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=300"),
        )],
        Html(html.to_string()),
    )
        .into_response()
}

/// `/install/` → `/install`; anything else unknown is the 404 page.
async fn fallback(uri: Uri) -> Response {
    let path = uri.path();
    if path.len() > 1 && path.ends_with('/') {
        return Redirect::permanent(path.trim_end_matches('/')).into_response();
    }
    (StatusCode::NOT_FOUND, Html(pages::not_found())).into_response()
}

/// `$PORT` as Cloud Run sets it; 8080 when unset, empty or not a number.
pub fn port_from_env(var: Option<&str>) -> u16 {
    var.and_then(|v| v.trim().parse().ok()).unwrap_or(8080)
}
