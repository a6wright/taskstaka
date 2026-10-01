//! The web UI: one shell (sidebar · list · side pane) rendered by
//! `assets/app.js` against the JSON API. The server only hands out the
//! shell and two static files; every route that looks like a page serves
//! the same shell and the script reads the URL.
//!
//! Mutations go through the JSON API, which takes JSON bodies only and sets
//! no CORS headers, so a cross-origin page cannot perform them (ADR 0003).

use std::sync::Arc;

use axum::Router;
use axum::http::header;
use axum::response::{Html, IntoResponse};
use axum::routing::get;

use super::AppState;

const APP_JS: &str = include_str!("../../assets/app.js");
const APP_CSS: &str = include_str!("../../assets/app.css");

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(shell))
        .route("/next", get(shell))
        .route("/waiting", get(shell))
        .route("/someday", get(shell))
        .route("/done", get(shell))
        .route("/t/{id}", get(shell))
        .route("/app.js", get(js))
        .route("/app.css", get(css))
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

async fn shell() -> Html<&'static str> {
    Html(SHELL)
}

async fn js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        APP_JS,
    )
}

async fn css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        APP_CSS,
    )
}

const SHELL: &str = r#"<!doctype html>
<html lang="en" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark light">
<title>taskstaka</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&display=swap">
<link rel="stylesheet" href="/app.css">
</head>
<body>
<div id="app">
  <aside id="side"></aside>
  <button id="side-toggle" title="collapse sidebar (\)">‹</button>
  <section id="main"><div class="content"><p class="empty">loading…</p></div></section>
  <button id="pane-toggle" title="close details (Esc)">›</button>
  <aside id="pane"></aside>
</div>
<script src="/app.js"></script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(
            esc(r#"<a href="x">&'"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
    }

    #[test]
    fn shell_references_assets_and_assets_are_nonempty() {
        assert!(SHELL.contains("/app.js") && SHELL.contains("/app.css"));
        assert!(APP_JS.contains("/api/v1/tree"));
        assert!(APP_CSS.contains("--accent"));
    }
}
