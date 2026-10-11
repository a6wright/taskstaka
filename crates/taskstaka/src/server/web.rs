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
const FAVICON: &[u8] = include_bytes!("../../assets/favicon.ico");
const TOUCH_ICON: &[u8] = include_bytes!("../../assets/apple-touch-icon.png");
const MARK: &[u8] = include_bytes!("../../assets/mark.png");

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(shell))
        .route("/next", get(shell))
        .route("/waiting", get(shell))
        .route("/someday", get(shell))
        .route("/done", get(shell))
        .route("/nudges", get(shell))
        .route("/settings", get(shell))
        .route("/stack", get(shell))
        .route("/quick", get(|| async { Html(QUICK) }))
        .route("/t/{id}", get(shell))
        .route("/favicon.ico", get(|| image("image/x-icon", FAVICON)))
        .route(
            "/apple-touch-icon.png",
            get(|| image("image/png", TOUCH_ICON)),
        )
        .route("/mark.png", get(|| image("image/png", MARK)))
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

/// Brand images change rarely and never need the no-cache the scripts get.
async fn image(ctype: &'static str, bytes: &'static [u8]) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, ctype),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        bytes,
    )
}

/// A bare capture box for a launcher or a small window: type a line, Enter
/// adds it as a new root task. `?context=personal` picks the context.
/// `?panel=1` is the tray's Spotlight-style window: transparent body, one
/// floating card, Esc/an add tell the host over wry IPC to hide the panel,
/// and `window.taskstakaReset()` clears and refocuses when it is shown again.
const QUICK: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Quick add</title><link rel="icon" href="/favicon.ico"><link rel="stylesheet" href="/app.css">
<style>body{display:grid;place-items:center;min-height:100vh;margin:0}
.qa{width:min(640px,92vw)}.qa input{width:100%;font-size:18px;padding:14px 16px;border-radius:10px;border:1px solid var(--line);background:var(--bg-pane);outline:none}
.qa input:focus{border-color:var(--accent)}.qa p{margin:10px 4px 0;color:var(--muted);font-size:12.5px;min-height:1.4em}.qa .ok{color:#22c55e}.qa .err{color:var(--danger)}
html.panel,html.panel body{background:transparent;min-height:0;height:100%;overflow:hidden}
html.panel body{display:block;padding:0}
html.panel .qa{width:100%;box-sizing:border-box;height:100vh;padding:0 14px;display:flex;align-items:center;gap:12px;border-radius:14px;background:color-mix(in srgb,var(--bg-pane) 92%,transparent);border:1px solid var(--line);box-shadow:0 18px 50px rgba(0,0,0,.35);backdrop-filter:blur(18px)}
html.panel .qa input{flex:1;font-size:22px;padding:10px 4px;border:0;background:transparent;border-radius:0}
html.panel .qa input:focus{border:0}
html.panel .qa p{margin:0;flex:0 0 auto;max-width:34%;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;text-align:right}</style></head>
<body><div class="qa"><input id="q" autofocus autocomplete="off" spellcheck="false" placeholder="Add a task…  (@Name waits on them · #project)">
<p id="m">Enter adds it as a new root task · Esc clears</p></div>
<script>
const q = document.getElementById('q'), m = document.getElementById('m');
const sp = new URLSearchParams(location.search);
const ctx = sp.get('context');
const panel = sp.get('panel') === '1';
if (panel) { document.documentElement.classList.add('panel'); m.textContent = ''; }
// The tray hosts this page in a wry webview; tell it when to hide the panel.
const host = (msg) => { if (panel && window.ipc) window.ipc.postMessage(msg); };
window.taskstakaReset = () => { q.value = ''; m.textContent = ''; m.className = ''; q.focus(); };
q.addEventListener('keydown', async (e) => {
  if (e.key === 'Escape') { if (panel) host('close'); else q.value = ''; return; }
  if (e.key !== 'Enter' || !q.value.trim()) return;
  e.preventDefault();
  try {
    const r = await fetch('/api/v1/quick', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ text: q.value, context: ctx }) });
    const j = await r.json();
    if (!r.ok) throw new Error((j.error && j.error.message) || j.message || r.statusText);
    m.className = 'ok'; m.textContent = 'added: ' + q.value; q.value = '';
    if (panel) { host('added'); setTimeout(() => host('close'), 650); }
  } catch (err) { m.className = 'err'; m.textContent = err.message; }
});
</script></body></html>"#;

const SHELL: &str = r#"<!doctype html>
<html lang="en" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark light">
<title>taskstaka</title>
<link rel="icon" href="/favicon.ico" sizes="any">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&display=swap">
<link rel="stylesheet" href="/app.css">
</head>
<body>
<div id="app">
  <aside id="side"></aside>
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
