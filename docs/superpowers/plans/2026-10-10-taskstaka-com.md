# taskstaka.com Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship https://taskstaka.com as a Rust service on Cloud Run: home with the install command, Install, Help, Changelog, with the app's look, three animated SVG diagrams and real screenshots.

**Architecture:** A new crate `crates/site` (binary `taskstaka-site`) renders Markdown from `crates/site/content/*.md` into HTML pages once at startup, wraps them in a hand-built shell (the same approach as the daemon's `web.rs`), and serves pages and embedded assets with axum. One container, built from the workspace root with a multi-stage Dockerfile, deployed with `gcloud run deploy --source`, domain mapped to `taskstaka.com`. Sync endpoints are added to this binary later.

**Tech Stack:** Rust 2024 workspace, axum 0.8, tokio, pulldown-cmark 0.13, tracing; Playwright (headless, from `e2e/node_modules`) for screenshots; Docker; Google Cloud Run (us-central1), project `taskstaka`.

**Spec:** `docs/superpowers/specs/2026-10-10-taskstaka-com-design.md`

## Global Constraints

- Commit identity: every commit as `54049107+a6wright@users.noreply.github.com` (repo-local config is set; verify with `git var GIT_AUTHOR_IDENT`).
- Builds on this Mac need `export DEVELOPER_DIR=/Library/Developer/CommandLineTools` (Xcode licence not accepted).
- `cargo fmt --all && cargo clippy --all-targets && cargo test` clean before each commit (CLAUDE.md).
- No OS or GUI code in `crates/site`; it is a plain HTTP service. No tokio/axum in `crates/core` (unchanged).
- Accent colour is the logo's orange `#e87818`; everything else uses the tokens from `crates/taskstaka/assets/app.css` (`--bg #0f1012`, `--bg-side #141518`, `--line #24262b`, `--fg #e8e9eb`, `--fg-2 #b4b6bb`, `--muted #80838a`, `--warn #e5b45b`, `--ok #4cc38a`), Inter via Google Fonts with the system fallback stack.
- Pages: `/`, `/install`, `/help`, `/changelog`, `/healthz`; assets under `/site.css`, `/logo.png`, `/mark.png`, `/favicon.ico`, `/apple-touch-icon.png`, `/shots/*.png`. Unknown paths → 404 in the shell.
- Container listens on `$PORT` (default 8080), runs as non-root, `--locked` build.
- Region `us-central1`, service name `taskstaka-site`, `--allow-unauthenticated`, `--min-instances 0`.

## Review Focus

- A heading repeated on one page (two "Updating" sections) must get distinct anchors, not a duplicate `id` — Task 2 tests `with_heading_ids` suffixes `-2`.
- `/install/` with a trailing slash and `/INSTALL` must not 404 silently — Task 2 normalises a trailing slash (redirect 308 to the bare path); case is left strict (test documents it).
- `PORT` unset, empty or non-numeric must fall back to 8080, never panic — Task 1 tests `port_from_env`.
- `HEAD /` and `GET /healthz` from Cloud Run's health checks must answer 200 fast — Task 1 and Task 7 test both.
- `prefers-reduced-motion` must show the diagrams' final state, not a blank or mid-animation frame — Task 5 tests each SVG contains the reduced-motion rule and the final-frame class.

---

## File structure

```
crates/site/
  Cargo.toml                 package taskstaka-site, bin taskstaka-site
  Dockerfile                 multi-stage, built from the repo root
  src/main.rs                env → bind → axum::serve; tracing to stdout
  src/lib.rs                 pub fn router() -> Router; pub fn port_from_env()
  src/pages.rs               Markdown → Page { path, title, html }; shell(); with_heading_ids()
  src/assets.rs              embedded bytes + content types + cache headers
  src/diagrams.rs            the three SVG strings, substituted into pages
  assets/site.css
  assets/diagrams/{tree,stack,resurfaced}.svg
  assets/shots/{next,outline}.png     captured by shots.js
  content/{index,install,help,changelog}.md
  shots.js                   headless screenshots of a scratch daemon (uses ../../e2e/node_modules)
  tests/site.rs              binds the router on port 0, drives it with ureq
.dockerignore                target/, e2e/node_modules, .superpowers, notes
```

Workspace `Cargo.toml` gains `pulldown-cmark = "0.13"` under `[workspace.dependencies]`.

---

### Task 1: Crate skeleton, `/healthz`, PORT

**Files:**
- Create: `crates/site/Cargo.toml`, `crates/site/src/main.rs`, `crates/site/src/lib.rs`, `crates/site/tests/site.rs`
- Modify: `Cargo.toml` (workspace deps)

**Interfaces:**
- Produces: `taskstaka_site::router() -> axum::Router`, `taskstaka_site::port_from_env(var: Option<&str>) -> u16`, test helper `start()` in `tests/site.rs` returning `(String /*base*/, JoinHandle)`.

- [ ] **Step 1: Manifest and workspace dep**

`crates/site/Cargo.toml`:
```toml
[package]
name = "taskstaka-site"
description = "taskstaka.com: the help site, and later the sync service. A plain HTTP service."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[lib]
path = "src/lib.rs"

[[bin]]
name = "taskstaka-site"
path = "src/main.rs"

[dependencies]
axum.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
pulldown-cmark.workspace = true

[dev-dependencies]
ureq.workspace = true
```
In the root `Cargo.toml` `[workspace.dependencies]` add, after `ratatui`:
```toml
pulldown-cmark = { version = "0.13", default-features = false, features = ["html"] }
```

- [ ] **Step 2: Failing tests**

`crates/site/tests/site.rs`:
```rust
//! Drive the real router in-process on a free port, the way tests/web.rs does for the daemon.
use std::time::Duration;

use taskstaka_site::router;

async fn start() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, router()).await.unwrap();
    });
    let agent = ureq::Agent::new_with_defaults();
    for _ in 0..50 {
        if agent.get(format!("{base}/healthz")).call().is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (base, handle)
}

fn get(base: &str, path: &str) -> (u16, String, String) {
    let agent = ureq::Agent::new_with_defaults();
    let mut res = agent
        .get(format!("{base}{path}"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    let status = res.status().as_u16();
    let ctype = res
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = res.body_mut().read_to_string().unwrap();
    (status, ctype, body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_answers_ok() {
    let (base, handle) = start().await;
    let (status, _, body) = get(&base, "/healthz");
    assert_eq!((status, body.as_str()), (200, "ok"));
    handle.abort();
}

#[test]
fn port_falls_back_to_8080() {
    use taskstaka_site::port_from_env;
    assert_eq!(port_from_env(None), 8080);
    assert_eq!(port_from_env(Some("")), 8080);
    assert_eq!(port_from_env(Some("abc")), 8080);
    assert_eq!(port_from_env(Some("9090")), 9090);
}
```

- [ ] **Step 3: Run, expect failure**

Run: `cargo test -p taskstaka-site`
Expected: compile error, `router` and `port_from_env` not found.

- [ ] **Step 4: Implement**

`crates/site/src/lib.rs`:
```rust
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
```
`crates/site/src/main.rs`:
```rust
use taskstaka_site::{port_from_env, router};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let port = port_from_env(std::env::var("PORT").ok().as_deref());
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("bind");
    tracing::info!(port, "taskstaka-site listening");
    axum::serve(listener, router()).await.expect("serve");
}
```

- [ ] **Step 5: Run, expect pass**

Run: `cargo test -p taskstaka-site`
Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets && git add Cargo.toml Cargo.lock crates/site && git commit -m "Site: taskstaka-site crate skeleton with /healthz and PORT"
```

---

### Task 2: Markdown pages and the shell

**Files:**
- Create: `crates/site/src/pages.rs`, `crates/site/content/index.md`, `install.md`, `help.md`, `changelog.md` (headings only for now; Task 4 writes the content)
- Modify: `crates/site/src/lib.rs`, `crates/site/tests/site.rs`

**Interfaces:**
- Produces: `pages::with_heading_ids(md: &str) -> String`, `pages::render_markdown(md: &str) -> String`, `pages::shell(title: &str, path: &str, body: &str) -> String`, `pages::Page { path: &'static str, title: &'static str, html: String }`, `pages::all() -> Vec<Page>`; router serves each page at its path, redirects `/x/` → `/x` (308), 404 page in the shell.
- Consumes: Task 1 `router()`.

- [ ] **Step 1: Failing unit tests** (append to `pages.rs` as `#[cfg(test)] mod tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_get_slug_ids_and_duplicates_are_suffixed() {
        let md = "# Install\n\ntext\n\n## Updating\n\n## Updating\n\n### Claude Code\n";
        let out = with_heading_ids(md);
        assert!(out.contains("# Install {#install}"), "{out}");
        assert!(out.contains("## Updating {#updating}"), "{out}");
        assert!(out.contains("## Updating {#updating-2}"), "{out}");
        assert!(out.contains("### Claude Code {#claude-code}"), "{out}");
        // Fenced code is left alone even when a line starts with #.
        let fenced = "```\n# not a heading\n```\n";
        assert_eq!(with_heading_ids(fenced), fenced);
    }

    #[test]
    fn markdown_renders_headings_with_ids_and_code_blocks() {
        let html = render_markdown("## Updating\n\n```\ncargo install --path crates/taskstaka\n```\n");
        assert!(html.contains(r#"<h2 id="updating">Updating</h2>"#), "{html}");
        assert!(html.contains("<pre><code>cargo install --path crates/taskstaka\n</code></pre>"), "{html}");
    }

    #[test]
    fn shell_wraps_the_body_and_marks_the_current_nav_item() {
        let html = shell("Install", "/install", "<p>hi</p>");
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<title>Install · taskstaka</title>"));
        assert!(html.contains(r#"<a href="/install" aria-current="page">"#), "{html}");
        assert!(!html.contains(r#"<a href="/help" aria-current="page">"#));
        assert!(html.contains("<p>hi</p>"));
    }

    #[test]
    fn every_page_renders_from_its_content_file() {
        let pages = all();
        let paths: Vec<&str> = pages.iter().map(|p| p.path).collect();
        assert_eq!(paths, vec!["/", "/install", "/help", "/changelog"]);
        for p in &pages {
            assert!(p.html.contains("<main"), "{}", p.path);
        }
    }
}
```

- [ ] **Step 2: Failing route tests** (append to `tests/site.rs`)

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pages_serve_html_and_unknown_paths_get_a_404_page() {
    let (base, handle) = start().await;
    for path in ["/", "/install", "/help", "/changelog"] {
        let (status, ctype, body) = get(&base, path);
        assert_eq!(status, 200, "{path}");
        assert!(ctype.starts_with("text/html"), "{path}: {ctype}");
        assert!(body.contains("taskstaka"), "{path}");
    }
    let (status, ctype, body) = get(&base, "/nope");
    assert_eq!(status, 404);
    assert!(ctype.starts_with("text/html") && body.contains("Not found"), "{body}");
    handle.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trailing_slash_redirects_to_the_bare_path_and_case_is_strict() {
    let (base, handle) = start().await;
    let agent = ureq::Agent::new_with_defaults();
    let res = agent
        .get(format!("{base}/install/"))
        .config()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .call()
        .unwrap();
    assert_eq!(res.status().as_u16(), 308);
    assert_eq!(res.headers().get("location").unwrap().to_str().unwrap(), "/install");
    let (status, _, _) = get(&base, "/INSTALL");
    assert_eq!(status, 404, "paths are case-sensitive; the links on the site are lowercase");
    handle.abort();
}
```

- [ ] **Step 3: Run, expect failure**

Run: `cargo test -p taskstaka-site`
Expected: compile errors (`pages` module missing).

- [ ] **Step 4: Content stubs** — create the four files with their real top headings so `all()` has something to render (Task 4 fills them):

`crates/site/content/index.md`: `# A "what needs action next" task server, backed by markdown\n`
`crates/site/content/install.md`: `# Install\n\n## Homebrew\n\n## From source\n\n## macOS menu bar and ⌥Space\n\n## Claude Code\n\n## Always on\n\n## Updating\n`
`crates/site/content/help.md`: `# Help\n\n## The model\n\n## The CLI\n\n## The web\n\n## The TUI\n\n## The menu bar\n\n## Agents\n`
`crates/site/content/changelog.md`: `# Changelog\n\n## 0.1.2\n\n## 0.1.1\n\n## 0.1.0\n`

- [ ] **Step 5: Implement `pages.rs`**

```rust
//! Markdown in, pages out. Rendered once at startup; the shell is hand-built
//! like the daemon's web.rs so there is no template engine to learn.

use pulldown_cmark::{Options, Parser, html};

pub struct Page {
    pub path: &'static str,
    pub title: &'static str,
    pub html: String,
}

const CONTENT: [(&str, &str, &str); 4] = [
    ("/", "taskstaka", include_str!("../content/index.md")),
    ("/install", "Install", include_str!("../content/install.md")),
    ("/help", "Help", include_str!("../content/help.md")),
    ("/changelog", "Changelog", include_str!("../content/changelog.md")),
];

pub fn all() -> Vec<Page> {
    CONTENT
        .iter()
        .map(|(path, title, md)| Page {
            path,
            title,
            html: shell(title, path, &render_markdown(md)),
        })
        .collect()
}

/// Append `{#slug}` to every ATX heading outside fenced code, so anchors are
/// stable and the app can deep-link. Repeats get `-2`, `-3`, …
pub fn with_heading_ids(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + 64);
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    let mut fenced = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let hashes = line.chars().take_while(|&c| c == '#').count();
        if !fenced && (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') && !line.contains(" {#") {
            let text = line[hashes..].trim();
            let base = slug(text);
            let n = seen.entry(base.clone()).and_modify(|n| *n += 1).or_insert(1);
            let id = if *n == 1 { base } else { format!("{base}-{n}") };
            out.push_str(&format!("{line} {{#{id}}}\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn slug(text: &str) -> String {
    let mut s = String::new();
    let mut dash = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
            dash = false;
        } else if !dash && !s.is_empty() {
            s.push('-');
            dash = true;
        }
    }
    s.trim_end_matches('-').to_string()
}

pub fn render_markdown(md: &str) -> String {
    let md = with_heading_ids(md);
    let opts = Options::ENABLE_HEADING_ATTRIBUTES | Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut out = String::with_capacity(md.len() * 2);
    html::push_html(&mut out, Parser::new_ext(&md, opts));
    out
}

const NAV: [(&str, &str); 3] = [("/install", "Install"), ("/help", "Help"), ("/changelog", "Changelog")];

pub fn shell(title: &str, path: &str, body: &str) -> String {
    let nav: String = NAV
        .iter()
        .map(|(href, label)| {
            let cur = if *href == path { r#" aria-current="page""# } else { "" };
            format!(r#"<a href="{href}"{cur}>{label}</a>"#)
        })
        .collect();
    let full_title = if path == "/" { "taskstaka".to_string() } else { format!("{title} · taskstaka") };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="color-scheme" content="dark light">
<title>{full_title}</title>
<meta name="description" content="taskstaka: a 'what needs action next' task server backed by markdown. Install in one line; works from the CLI, the web, the TUI, the menu bar, and with Claude.">
<link rel="icon" href="/favicon.ico" sizes="any">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&display=swap">
<link rel="stylesheet" href="/site.css">
</head>
<body>
<header class="top"><a class="brand" href="/"><img src="/mark.png" alt="" width="26" height="26">taskstaka</a><nav>{nav}<a href="https://github.com/a6wright/taskstaka">GitHub</a></nav></header>
<main class="page">
{body}
</main>
<footer class="foot"><span>taskstaka · MIT</span><a href="https://github.com/a6wright/taskstaka/releases">releases</a></footer>
</body>
</html>
"#
    )
}

pub fn not_found() -> String {
    shell("Not found", "/404", "<h1>Not found</h1><p>There is no page here. Try <a href=\"/install\">Install</a> or <a href=\"/help\">Help</a>.</p>")
}
```

- [ ] **Step 6: Wire the router** (`lib.rs`)

```rust
use std::sync::Arc;

use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;

pub mod pages;

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
        [(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=300"))],
        Html(html.to_string()),
    )
        .into_response()
}

async fn fallback(uri: Uri) -> Response {
    let path = uri.path();
    if path.len() > 1 && path.ends_with('/') {
        return Redirect::permanent(path.trim_end_matches('/')).into_response();
    }
    (StatusCode::NOT_FOUND, Html(pages::not_found())).into_response()
}
```
Note: axum's `Redirect::permanent` is 308. `Html<String>` from an `Arc<str>` clone per request is fine at this size; pages are a few KB.

- [ ] **Step 7: Run, expect pass**

Run: `cargo test -p taskstaka-site`
Expected: all pass (4 unit, 4 integration).

- [ ] **Step 8: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets && git add crates/site && git commit -m "Site: Markdown pages with stable heading anchors, the shell, 404 and trailing-slash redirect"
```

---

### Task 3: Stylesheet and embedded assets

**Files:**
- Create: `crates/site/src/assets.rs`, `crates/site/assets/site.css`
- Modify: `crates/site/src/lib.rs`, `crates/site/tests/site.rs`

**Interfaces:**
- Produces: `assets::ASSETS: &[(&str /*path*/, &str /*content type*/, &[u8])]`, routes for each; `shots/*.png` entries are added by Task 6 to the same table.
- Consumes: brand files at `crates/taskstaka/assets/{mark.png,favicon.ico,apple-touch-icon.png}` and `assets/brand/logo.png` (via `include_bytes!` with relative paths from `crates/site/src/`).

- [ ] **Step 1: Failing test** (append to `tests/site.rs`)

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assets_have_types_and_cache_headers() {
    let (base, handle) = start().await;
    let agent = ureq::Agent::new_with_defaults();
    for (path, ctype) in [
        ("/site.css", "text/css"),
        ("/logo.png", "image/png"),
        ("/mark.png", "image/png"),
        ("/favicon.ico", "image/x-icon"),
        ("/apple-touch-icon.png", "image/png"),
    ] {
        let res = agent.get(format!("{base}{path}")).call().unwrap();
        assert_eq!(res.status().as_u16(), 200, "{path}");
        assert!(res.headers().get("content-type").unwrap().to_str().unwrap().starts_with(ctype), "{path}");
        assert_eq!(res.headers().get("cache-control").unwrap().to_str().unwrap(), "public, max-age=86400", "{path}");
    }
    let (_, _, css) = get(&base, "/site.css");
    assert!(css.contains("--accent: #e87818"), "the logo orange is the accent");
    assert!(css.contains("@media (max-width: 640px)"), "phone rules exist");
    handle.abort();
}
```

- [ ] **Step 2: Run, expect failure** — `cargo test -p taskstaka-site assets` → 404s.

- [ ] **Step 3: Implement `assets.rs`**

```rust
//! Everything the pages reference, embedded so the container is one file.

pub struct Asset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

pub const ASSETS: &[Asset] = &[
    Asset { path: "/site.css", content_type: "text/css; charset=utf-8", bytes: include_bytes!("../assets/site.css") },
    Asset { path: "/logo.png", content_type: "image/png", bytes: include_bytes!("../../../assets/brand/logo.png") },
    Asset { path: "/mark.png", content_type: "image/png", bytes: include_bytes!("../../taskstaka/assets/mark.png") },
    Asset { path: "/favicon.ico", content_type: "image/x-icon", bytes: include_bytes!("../../taskstaka/assets/favicon.ico") },
    Asset { path: "/apple-touch-icon.png", content_type: "image/png", bytes: include_bytes!("../../taskstaka/assets/apple-touch-icon.png") },
];
```
In `lib.rs` add `pub mod assets;` and, inside `router()` before the fallback:
```rust
    for a in assets::ASSETS {
        r = r.route(a.path, get(move || asset_response(a)));
    }
```
with
```rust
async fn asset_response(a: &'static assets::Asset) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(a.content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400")),
        ],
        a.bytes,
    )
        .into_response()
}
```

- [ ] **Step 4: Write `assets/site.css`** — identity-preserving tokens from `app.css`, the orange accent, one column, phone-first:

```css
/* taskstaka.com — the app's palette, the logo's orange as the one accent. */
:root {
  --bg: #0f1012; --bg-2: #141518; --bg-3: #1b1d21; --line: #24262b;
  --fg: #e8e9eb; --fg-2: #b4b6bb; --muted: #8a8d94;
  --accent: #e87818; --accent-2: #f8a848; --warn: #e5b45b; --ok: #4cc38a;
  --font: "Inter", ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  --mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  --measure: 68ch;
}
@media (prefers-color-scheme: light) {
  :root { --bg: #ffffff; --bg-2: #f7f7f8; --bg-3: #efeff2; --line: #e3e3e8; --fg: #1b1b1f; --fg-2: #3c3d42; --muted: #5f626a; --accent: #d26a0f; }
}
* { box-sizing: border-box; }
html { background: var(--bg); color: var(--fg); font: 16px/1.6 var(--font); -webkit-font-smoothing: antialiased; }
@media (min-width: 720px) { html { font-size: 17px; } }
body { margin: 0; min-height: 100vh; display: flex; flex-direction: column; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; text-underline-offset: 3px; }
code, pre { font-family: var(--mono); font-size: 0.92em; }
pre { background: var(--bg-2); border: 1px solid var(--line); border-radius: 10px; padding: 14px 16px; overflow-x: auto; line-height: 1.5; }
:not(pre) > code { background: var(--bg-3); padding: 1px 6px; border-radius: 5px; }

/* top bar */
.top { position: sticky; top: 0; z-index: 10; background: color-mix(in srgb, var(--bg) 88%, transparent); backdrop-filter: blur(10px); border-bottom: 1px solid var(--line); display: flex; align-items: center; gap: 16px; padding: 10px max(16px, env(safe-area-inset-left)); }
.brand { display: inline-flex; align-items: center; gap: 9px; color: var(--fg); font-weight: 600; letter-spacing: -0.01em; }
.brand:hover { text-decoration: none; }
.top nav { margin-left: auto; display: flex; gap: 2px; overflow-x: auto; scrollbar-width: none; }
.top nav a { color: var(--fg-2); padding: 6px 10px; border-radius: 6px; white-space: nowrap; }
.top nav a:hover { background: var(--bg-3); text-decoration: none; color: var(--fg); }
.top nav a[aria-current="page"] { color: var(--fg); background: var(--bg-3); }

/* page */
.page { width: min(760px, 100% - 32px); margin: 0 auto; padding: clamp(24px, 5vw, 56px) 0 80px; flex: 1; }
.page > p, .page > ul, .page > ol { max-width: var(--measure); }
h1 { font-size: clamp(1.75rem, 1.2rem + 2.4vw, 2.6rem); line-height: 1.15; letter-spacing: -0.02em; margin: 0 0 0.6em; text-wrap: balance; }
h2 { font-size: 1.45rem; letter-spacing: -0.015em; margin: 2.2em 0 0.5em; padding-top: 0.4em; border-top: 1px solid var(--line); }
h3 { font-size: 1.1rem; margin: 1.6em 0 0.4em; }
h2 a, h3 a { color: inherit; }
p, li { color: var(--fg-2); }
p strong, li strong { color: var(--fg); font-weight: 600; }
hr { border: 0; border-top: 1px solid var(--line); margin: 2.5em 0; }
img { max-width: 100%; height: auto; border-radius: 10px; border: 1px solid var(--line); display: block; }
table { border-collapse: collapse; width: 100%; font-size: 0.95em; }
th, td { text-align: left; padding: 7px 10px; border-bottom: 1px solid var(--line); vertical-align: top; }
th { color: var(--muted); font-weight: 500; }
kbd { font-family: var(--mono); font-size: 0.85em; background: var(--bg-3); border: 1px solid var(--line); border-radius: 5px; padding: 1px 6px; color: var(--fg); }

/* home */
.hero { text-align: center; padding: 8px 0 24px; }
.hero img.logo { width: min(260px, 60vw); margin: 0 auto 18px; border: 0; border-radius: 0; }
.hero h1 { max-width: 22ch; margin-inline: auto; }
.hero p { font-size: 1.1em; max-width: 52ch; margin: 0 auto 26px; }
.install { background: var(--bg-2); border: 1.5px solid var(--accent); border-radius: 12px; padding: 16px 18px; font-family: var(--mono); font-size: 1.05em; text-align: left; display: flex; gap: 12px; align-items: baseline; overflow-x: auto; max-width: 560px; margin: 0 auto; }
.install .prompt { color: var(--accent); user-select: none; }
.install + p { margin-top: 10px; font-size: 0.95em; color: var(--muted); }
.three { display: grid; gap: 22px 28px; margin: 40px 0; }
@media (min-width: 720px) { .three { grid-template-columns: repeat(3, 1fr); } }
.three h3 { margin: 0 0 4px; }
.three p { margin: 0; font-size: 0.97em; }

/* diagrams */
.diagram { margin: 28px 0; }
.diagram svg { width: 100%; height: auto; display: block; }
figcaption { color: var(--muted); font-size: 0.9em; margin-top: 8px; }

.foot { border-top: 1px solid var(--line); color: var(--muted); font-size: 0.9em; display: flex; gap: 16px; padding: 18px max(16px, env(safe-area-inset-left)) max(18px, env(safe-area-inset-bottom)); }
.foot a { color: var(--muted); }

@media (max-width: 640px) {
  .top { gap: 10px; }
  .brand { font-size: 0; gap: 0; }
  .page { width: min(760px, 100% - 24px); }
  pre { padding: 12px; border-radius: 8px; }
  .install { padding: 12px 14px; font-size: 0.95em; }
}
```

- [ ] **Step 5: Run, expect pass** — `cargo test -p taskstaka-site`.

- [ ] **Step 6: Commit** — `git add crates/site && git commit -m "Site: stylesheet with the app's tokens and the logo orange; embedded brand assets"`.

---

### Task 4: Content

**Files:**
- Modify: `crates/site/content/index.md`, `install.md`, `help.md`, `changelog.md`
- Modify: `crates/site/tests/site.rs`

**Interfaces:**
- Consumes: the anchors Task 2 generates from headings (`/help#the-model`, `/install#homebrew`).
- Produces: pages whose HTML contains the install command, the diagram placeholders (`<!-- diagram:tree -->`, `<!-- diagram:stack -->`, `<!-- diagram:resurfaced -->`, consumed by Task 5) and the screenshot `<img>`s (`/shots/next.png`, `/shots/outline.png`, served by Task 6).

- [ ] **Step 1: Failing test** (append to `tests/site.rs`)

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_has_the_install_command_anchors_and_placeholders() {
    let (base, handle) = start().await;
    let (_, _, home) = get(&base, "/");
    assert!(home.contains("brew install a6wright/taskstaka/taskstaka"), "install command on the home page");
    assert!(home.contains("/shots/next.png"));
    let (_, _, install) = get(&base, "/install");
    for id in ["homebrew", "from-source", "macos-menu-bar-and-space", "claude-code", "always-on", "updating"] {
        assert!(install.contains(&format!(r#"id="{id}""#)), "install anchor {id}");
    }
    let (_, _, help) = get(&base, "/help");
    for id in ["the-model", "the-cli", "the-web", "the-tui", "the-menu-bar", "agents"] {
        assert!(help.contains(&format!(r#"id="{id}""#)), "help anchor {id}");
    }
    assert!(help.contains("<svg") || help.contains("<!-- diagram:stack -->"), "diagram placeholder or SVG present");
    let (_, _, log) = get(&base, "/changelog");
    assert!(log.contains("0.1.2") && log.contains("0.1.0"));
    handle.abort();
}
```

- [ ] **Step 2: Run, expect failure** — anchors and the command are missing.

- [ ] **Step 3: Write the content.** Source: `README.md` (Install, Updating and running, Daily use, The model in short, The daemon, Other clients, Waiting on people, Agents) and `CONTEXT.md`. Rules: one block per heading, prose in short paragraphs, every command in a fenced block, keys in `<kbd>`. Raw HTML is allowed in the Markdown (pulldown passes it through), used for the hero and the diagram placeholders.

`index.md`:
````markdown
<div class="hero">
<img class="logo" src="/logo.png" alt="A pyramid of orange cubes with two tumbling off">

# A "what needs action next" task server, backed by markdown

<p>Work is a tree: a story spawns subtasks, the subtasks block the story. taskstaka keeps the tree and tells you the one thing that is yours to do right now — and brings the story back the moment its last piece is done.</p>

<div class="install"><span class="prompt">$</span><span>brew install a6wright/taskstaka/taskstaka</span></div>
<p>then <code>taskstaka init</code>. macOS, Linux. <a href="/install">Other ways to install</a>.</p>
</div>

<img src="/shots/next.png" alt="The Next view: the task in hand on top with its notes, then each story as a stack of what you can do now" width="1280" height="800">

<div class="three">
<div><h3>A tree, not a list</h3><p>Nesting means <em>first this</em>. A task with open subtasks is blocked and stays out of your way; the deepest open leaves are what you see.</p></div>
<div><h3>The plate</h3><p>The task in hand sits on top with its notes. Finish it and the next one in the same story takes its place. Put it back and a different one comes up.</p></div>
<div><h3>Waiting on people</h3><p>A wait always has a follow-up date. Chase, record what they said, and the task that depends on them is the one that waits — not the story.</p></div>
</div>

## How a stack works

<!-- diagram:stack -->

A story is the base. Each subtask under it is a prerequisite, knocked out from the bottom. When you tick the plate in hand, the next prerequisite moves into your hand; when the last one closes, the story itself comes back to the top of Next, flagged **resurfaced**. That moment is the whole product. More in [Help](/help#the-model).

## Everywhere you work

`taskstaka next` in a terminal, `taskstaka tui` as an always-on panel, the web outliner at `http://127.0.0.1:7407/`, the macOS menu bar item with <kbd>⌥Space</kbd> quick add, and an MCP server so Claude can keep your list honest. One daemon, one set of markdown files in `~/todo`, every surface showing the same thing. Your notes stay yours: open the folder in Obsidian, sync it with git.
````

`install.md` (abridged here; the implementer copies each command verbatim from README "Install", "Updating and running", "The daemon", "Other clients"):
````markdown
# Install

## Homebrew

macOS and Linux. Builds from source, so the first install takes a few minutes.

```
brew install a6wright/taskstaka/taskstaka
taskstaka init                 # creates ~/todo (or $TASKSTAKA_DIR)
```

## From source

```
git clone https://github.com/a6wright/taskstaka
cd taskstaka
cargo install --path crates/taskstaka
taskstaka init
```

## macOS menu bar and ⌥Space

The menu bar item shows the task in hand, its notes, and what you can do now; <kbd>⌥Space</kbd> opens quick add from any app.

```
cargo install --path crates/tray      # taskstaka-tray (brew installs it already)
taskstaka setup macos                 # launchd agents for the daemon and the tray
```

`taskstaka setup macos --hotkey ctrl+alt+space` picks another key; `--uninstall` removes both agents.

## Claude Code

```
taskstaka setup claude
```

Installs the taskstaka skill, adds a managed block to `~/.claude/CLAUDE.md`, and registers the MCP server. Restart Claude Code or run `/mcp`, then ask "what should I work on?".

## Always on

The first command starts the daemon for you. To keep it across logins:

```
brew services start taskstaka                 # Homebrew, daemon only
taskstaka setup macos                         # macOS, daemon and menu bar
systemctl --user enable --now taskstaka       # Linux, with the unit from the README
```

## Updating

```
brew upgrade taskstaka && taskstaka restart
```

From a checkout: `cargo install --path crates/taskstaka` (and `crates/tray`), then `taskstaka restart`; on macOS `taskstaka setup macos` again reloads the tray.
````

`help.md`: sections `# Help`, `## The model` (task, nesting = first this, siblings = and also, blocked/resurfaced, Next, the plate / in hand / up next / put back, waiting on with follow-ups and chase, someday — each as a short paragraph, with `<!-- diagram:tree -->` after the nesting paragraph and `<!-- diagram:resurfaced -->` after the resurfaced paragraph), `## The CLI` (a table: `add`, `next`, `start`, `done`, `wait`, `chase`, `reply`, `someday`, `tui`, `quick`, `restart`, `status` with one line each from README "Daily use"), `## The web` (outline keys <kbd>Enter</kbd> <kbd>Tab</kbd> <kbd>⇧Tab</kbd>, `@Name +3d`, `#project`, <kbd>n</kbd> quick add, the pane, `<img src="/shots/outline.png" …>`), `## The TUI` (views 1–5, keys from README), `## The menu bar` (title, menu, ⌥Space panel), `## Agents` (MCP tools list and the plan habit, from README "Agents").

`changelog.md`: `# Changelog` then `## 0.1.2 — 2026-10-10`, `## 0.1.1 — 2026-10-10`, `## 0.1.0 — 2026-10-10` with the release notes used for the GitHub releases (bullets).

- [ ] **Step 4: Run, expect pass** — `cargo test -p taskstaka-site`.

- [ ] **Step 5: Commit** — `git add crates/site/content crates/site/tests && git commit -m "Site: content for home, install, help and changelog"`.

---

### Task 5: Animated SVG diagrams

**Files:**
- Create: `crates/site/src/diagrams.rs`, `crates/site/assets/diagrams/tree.svg`, `stack.svg`, `resurfaced.svg`
- Modify: `crates/site/src/pages.rs` (substitute placeholders after rendering)

**Interfaces:**
- Produces: `diagrams::inline(html: &str) -> String` replacing `<!-- diagram:NAME -->` with `<figure class="diagram">` + the SVG.
- Consumes: Task 4 placeholders.

- [ ] **Step 1: Failing tests** (in `diagrams.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_become_inline_svg_figures() {
        let html = inline("<p>a</p>\n<!-- diagram:stack -->\n<p>b</p>");
        assert!(html.contains(r#"<figure class="diagram" id="diagram-stack">"#), "{html}");
        assert!(html.contains("<svg"), "{html}");
        assert!(!html.contains("<!-- diagram:stack -->"));
        assert_eq!(inline("<!-- diagram:nope -->"), "<!-- diagram:nope -->", "unknown names are left for the author to notice");
    }

    #[test]
    fn every_diagram_animates_without_js_and_respects_reduced_motion() {
        for (name, svg) in ALL {
            assert!(svg.contains("<style>"), "{name} has its own styles");
            assert!(svg.contains("@keyframes"), "{name} animates");
            assert!(svg.contains("prefers-reduced-motion"), "{name} has a still frame");
            assert!(svg.contains(r#"role="img""#) && svg.contains("<title>"), "{name} is labelled");
            assert!(!svg.contains("<script"), "{name} has no script");
        }
    }
}
```

- [ ] **Step 2: Run, expect failure** — module missing.

- [ ] **Step 3: Implement `diagrams.rs`**

```rust
//! The three diagrams that explain stacks, inline SVG with CSS animation so
//! they move without JavaScript and sit still under prefers-reduced-motion.

pub const ALL: &[(&str, &str)] = &[
    ("tree", include_str!("../assets/diagrams/tree.svg")),
    ("stack", include_str!("../assets/diagrams/stack.svg")),
    ("resurfaced", include_str!("../assets/diagrams/resurfaced.svg")),
];

const CAPTIONS: &[(&str, &str)] = &[
    ("tree", "A story spawns subtasks. While they are open, the story is blocked and out of your way."),
    ("stack", "Next shows the story as a stack. Tick the plate in hand and the next prerequisite moves into it."),
    ("resurfaced", "When the last subtask closes, the story comes back to the top of Next, flagged ready."),
];

/// Replace `<!-- diagram:NAME -->` with the figure. Unknown names are left in place.
pub fn inline(html: &str) -> String {
    let mut out = html.to_string();
    for (name, svg) in ALL {
        let marker = format!("<!-- diagram:{name} -->");
        let caption = CAPTIONS.iter().find(|(n, _)| n == name).map(|(_, c)| *c).unwrap_or("");
        let figure = format!(
            "<figure class=\"diagram\" id=\"diagram-{name}\">{svg}<figcaption>{caption}</figcaption></figure>"
        );
        out = out.replace(&marker, &figure);
    }
    out
}
```
In `pages::all()`, change `html: shell(title, path, &render_markdown(md))` to `html: shell(title, path, &crate::diagrams::inline(&render_markdown(md)))`, and add `pub mod diagrams;` to `lib.rs`.

- [ ] **Step 4: Draw the SVGs.** Shared conventions: `viewBox="0 0 720 300"`, `role="img"`, `<title>`, a `<style>` block scoped by a unique root class (`.d-tree`, `.d-stack`, `.d-res`) so three inline SVGs on one page do not fight, fills from CSS variables with fallbacks (`fill="var(--bg-2, #141518)"`), Inter via `font-family: inherit`, animations with `cubic-bezier(.2,.7,.2,1)` 600 ms moves and 1.2 s holds, looping with a total cycle of 8–10 s, and:
```css
@media (prefers-reduced-motion: reduce) { .d-stack * { animation: none !important; } .d-stack .final { opacity: 1; } }
```
Each SVG ends on a readable final state when animation is off (the `.final` elements are the finished picture).

`stack.svg` (the one on the home page): four rounded "plates" stacked (x 160–560, heights 48, radius 10, stroke `var(--line)`), labels left (`Renew the cert`, `Ask Priya for the DNS record`, `Restart nginx`) and the story as the base plate `so that · Ship the new site` with a heavier top border in `var(--accent)` when it lights up. Timeline (one `@keyframes` per plate, same duration, different delays): plate 1 gets a tick ✓ (`--ok`) at 1 s and slides left and fades by 1.6 s; plates 2–3 move down 56 px by 2.2 s; plate 2 gets the `in hand` badge (`--warn`); repeat for plates 2 and 3 at 3.5 s and 6 s; at 8 s the base plate's border turns `var(--accent)` with the badge `↑ ready`; hold 1.5 s; loop.

`tree.svg`: the story box at top (`Ship the new site`), three subtask boxes appear below one by one (0.4 s apart, fade + 8 px rise), lines drawn from the story to each (stroke-dashoffset animation), the story dims to 45% opacity with a `blocked` label when the first child appears; hold; loop.

`resurfaced.svg`: a short Next list (three rows) on the left and the story box greyed at the bottom; the last open subtask row gets ticked; the story box animates up to the top row with an `↑ ready` badge in `var(--accent)` and a brief glow (`filter: drop-shadow` fading out); hold; loop.

- [ ] **Step 5: Run, expect pass** — `cargo test -p taskstaka-site`.

- [ ] **Step 6: Look at them.** `cargo run -p taskstaka-site` then open `http://127.0.0.1:8080/` and `/help` in a browser (or the headless shots in Task 6) and watch each full cycle once; fix anything that overlaps text or ends mid-move.

- [ ] **Step 7: Commit** — `git add crates/site && git commit -m "Site: three animated SVG diagrams — tree, stack, resurfaced — inline with reduced-motion stills"`.

---

### Task 6: Screenshots of the app for the site

**Files:**
- Create: `crates/site/shots.js`, `crates/site/assets/shots/next.png`, `outline.png`
- Modify: `crates/site/src/assets.rs` (two entries), `crates/site/tests/site.rs` (extend `assets_have_types_and_cache_headers` with `/shots/next.png` and `/shots/outline.png`)

**Interfaces:**
- Produces: `/shots/next.png`, `/shots/outline.png` (1280×800, dark).
- Consumes: `target/release/taskstaka` built from the workspace; `e2e/node_modules/playwright`.

- [ ] **Step 1: Extend the assets test** — add `("/shots/next.png", "image/png"), ("/shots/outline.png", "image/png")` to the loop in `assets_have_types_and_cache_headers`. Run: fails with 404.

- [ ] **Step 2: Write `crates/site/shots.js`** (pattern from `e2e/run.js`: scratch dir, `init`, `serve --port 0 --no-notify`, read `daemon.json` for the port):

```js
// Screenshots of a demo tree for the site. Run from the repo root after
// `cargo build --release -p taskstaka`:  node crates/site/shots.js
const { chromium } = require('../../e2e/node_modules/playwright');
const { spawn, execFileSync } = require('child_process');
const fs = require('fs'), os = require('os'), path = require('path');

const BIN = path.resolve(__dirname, '..', '..', 'target', 'release', 'taskstaka');
const OUT = path.join(__dirname, 'assets', 'shots');
fs.mkdirSync(OUT, { recursive: true });
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'taskstaka-site-'));
const env = { ...process.env, TASKSTAKA_DIR: dir, TASKSTAKA_NO_NOTIFY: 'true', TASKSTAKA_DIRECT: '1' };
const ts = (...a) => execFileSync(BIN, a, { env }).toString().trim();
const id = (s) => s.split(/\s+/)[1];

ts('init');
const story = id(ts('add', 'Ship the new site', '--project', 'website'));
const dns = id(ts('add', '-p', story, 'Point taskstaka.com at Cloud Run'));
ts('add', '-p', dns, 'Ask Priya for the DNS record', '-w', 'Priya');
const cert = id(ts('add', '-p', story, 'Renew the cert'));
ts('touch', cert, 'certbot renew on web-1, then restart nginx and check the expiry in the browser');
ts('start', cert);
ts('add', '-p', story, 'Write the launch note');
const inbox = id(ts('add', 'Reply to the accountant', '--project', 'admin'));
ts('add', '-p', inbox, 'Find the Q3 receipts');
ts('add', 'Learn the ukulele', '--someday');

delete env.TASKSTAKA_DIRECT;
const daemon = spawn(BIN, ['serve', '--port', '0', '--no-notify'], { env, stdio: ['ignore', 'ignore', 'inherit'] });
(async () => {
  const f = path.join(dir, '.taskstaka', 'daemon.json');
  let port;
  for (let i = 0; i < 100 && !port; i++) { try { port = JSON.parse(fs.readFileSync(f, 'utf8')).port; } catch (_) { await new Promise(r => setTimeout(r, 50)); } }
  const base = `http://127.0.0.1:${port}`;
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, colorScheme: 'dark' });
  for (const [name, p] of [['next', '/next'], ['outline', '/']]) {
    await page.goto(base + p, { waitUntil: 'load' });
    await page.waitForTimeout(800);
    await page.screenshot({ path: path.join(OUT, name + '.png') });
  }
  await browser.close();
  daemon.kill();
  console.log('wrote', fs.readdirSync(OUT).join(' '));
})().catch(e => { console.error(e); daemon.kill(); process.exit(1); });
```

- [ ] **Step 3: Capture** — `cargo build --release -p taskstaka && node crates/site/shots.js`. Open both PNGs (Read tool) and check: the Next view shows the in-hand plate with notes and a story stack; nothing personal in the data (it is the scratch tree above).

- [ ] **Step 4: Embed** — add to `assets::ASSETS`:
```rust
    Asset { path: "/shots/next.png", content_type: "image/png", bytes: include_bytes!("../assets/shots/next.png") },
    Asset { path: "/shots/outline.png", content_type: "image/png", bytes: include_bytes!("../assets/shots/outline.png") },
```

- [ ] **Step 5: Run, expect pass** — `cargo test -p taskstaka-site`.

- [ ] **Step 6: Commit** — `git add crates/site && git commit -m "Site: screenshots of a demo tree (Next and Outline), captured headless by shots.js"`.

---

### Task 7: Container

**Files:**
- Create: `crates/site/Dockerfile`, `.dockerignore`
- Modify: `crates/site/tests/site.rs` (HEAD test)

**Interfaces:**
- Produces: an image that serves on `$PORT`; `docker build -f crates/site/Dockerfile .` from the repo root.

- [ ] **Step 1: Failing test** — Cloud Run probes with GET, browsers with HEAD; make sure HEAD works:
```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_requests_answer_like_get() {
    let (base, handle) = start().await;
    let agent = ureq::Agent::new_with_defaults();
    let res = agent.head(format!("{base}/")).call().unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert!(res.headers().get("content-type").unwrap().to_str().unwrap().starts_with("text/html"));
    handle.abort();
}
```
Run: expect pass already (axum answers HEAD for GET routes); if it fails, add `.route(path, get(..).head(..))`. Keep the test either way.

- [ ] **Step 2: `.dockerignore`** at the repo root:
```
target
e2e/node_modules
e2e/shots
.superpowers
.git
**/*.md.bak
```

- [ ] **Step 3: `crates/site/Dockerfile`**
```dockerfile
# Build from the repo root:  docker build -f crates/site/Dockerfile -t taskstaka-site .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY assets ./assets
RUN cargo build --release --locked -p taskstaka-site

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
 && useradd -r -u 10001 site
COPY --from=build /src/target/release/taskstaka-site /usr/local/bin/taskstaka-site
USER site
ENV PORT=8080 RUST_LOG=info
EXPOSE 8080
CMD ["taskstaka-site"]
```
Note: `COPY crates ./crates` brings the whole workspace (the daemon, tray crates) because `--locked -p taskstaka-site` still needs every member's manifest to resolve the workspace. The tray's GUI deps are not compiled (not a dependency of the site).

- [ ] **Step 4: Build and run locally**
```bash
docker build -f crates/site/Dockerfile -t taskstaka-site . && docker run --rm -d -p 8080:8080 --name site taskstaka-site && sleep 1 && curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/ && curl -s http://127.0.0.1:8080/healthz && docker stop site
```
Expected: `200`, `ok`.

- [ ] **Step 5: Commit** — `git add .dockerignore crates/site && git commit -m "Site: Dockerfile (multi-stage, non-root, PORT) and .dockerignore"`.

---

### Task 8: Deploy and the domain

**Files:**
- Modify: `README.md` (one line under Install pointing at taskstaka.com; Repo layout gains `crates/site`), `docs/PLAN.md` (P25 row + Resume here), `CLAUDE.md` (Layout line for `crates/site`)
- taskstaka store: the tasks `tg51vn` (deploy), `5va4z3` (mapping), `fndzy6` (DNS, user), `yq1ttt` (verify)

- [ ] **Step 1: Deploy**
```bash
gcloud run deploy taskstaka-site --source . --region us-central1 --project taskstaka \
  --allow-unauthenticated --min-instances 0 --cpu 1 --memory 256Mi --port 8080 \
  --dockerfile crates/site/Dockerfile 2>&1 | tail -5
```
If `--dockerfile` is not accepted by this gcloud, build and push explicitly:
```bash
gcloud artifacts repositories create site --repository-format docker --location us-central1 --project taskstaka
gcloud builds submit --project taskstaka --tag us-central1-docker.pkg.dev/taskstaka/site/taskstaka-site:0.1.2 --config /dev/stdin . <<'EOF'
steps: [{ name: 'gcr.io/cloud-builders/docker', args: ['build', '-f', 'crates/site/Dockerfile', '-t', 'us-central1-docker.pkg.dev/taskstaka/site/taskstaka-site:0.1.2', '.'] }]
images: ['us-central1-docker.pkg.dev/taskstaka/site/taskstaka-site:0.1.2']
EOF
gcloud run deploy taskstaka-site --image us-central1-docker.pkg.dev/taskstaka/site/taskstaka-site:0.1.2 --region us-central1 --project taskstaka --allow-unauthenticated --min-instances 0 --cpu 1 --memory 256Mi --port 8080
```
Expected: a `*.run.app` URL. `curl -s -o /dev/null -w '%{http_code}' <url>/` → 200. Record the URL in task `tg51vn`'s notes and close it.

- [ ] **Step 2: Domain mapping**
```bash
gcloud beta run domain-mappings create --service taskstaka-site --domain taskstaka.com --region us-central1 --project taskstaka
gcloud beta run domain-mappings create --service taskstaka-site --domain www.taskstaka.com --region us-central1 --project taskstaka
gcloud beta run domain-mappings describe --domain taskstaka.com --region us-central1 --project taskstaka --format='yaml(status.resourceRecords)'
```
If the domain is not yet verified for this account, the first command says so: run `gcloud domains verify taskstaka.com`, which opens Search Console; the TXT record it asks for becomes a sub-step of the user's DNS task. Write the returned records (A ×4, AAAA ×4 for `@`; CNAME `www` → `ghs.googlehosted.com`) into task `fndzy6`'s body as Namecheap Advanced DNS rows, close `5va4z3`, and tell the user the DNS step is theirs.

- [ ] **Step 3: Docs** — README Install gains a first line `Docs and install: https://taskstaka.com`; Repo layout gains `- crates/site — taskstaka.com (and later the sync service): Markdown pages, Cloud Run.`; CLAUDE.md Layout gains the same line; PLAN.md gets row `P25 taskstaka.com` and the Resume section notes the DNS step pending with the user and the deploy workflow (GitHub Action with Workload Identity) as the next step.

- [ ] **Step 4: Commit and push** — `git add README.md CLAUDE.md docs && git commit -m "Docs: taskstaka.com (crates/site on Cloud Run), PLAN P25" && git push origin main`.

- [ ] **Step 5: Verify when DNS lands** — after the user adds the records: `curl -sI https://taskstaka.com | head -3` → 200 with a valid certificate (Cloud Run provisions it, up to 15 minutes after DNS resolves). Close `yq1ttt` with the result in its notes; the story resurfaces as done.

---

## Self-review

- Spec coverage: pages (T2, T4), diagrams (T5), look (T3), screenshots (T6), tech and container (T1, T7), deploy and domain (T8), testing (every task), non-goals untouched. Changelog regeneration action and deploy action are explicitly follow-ups in the spec and recorded in PLAN "Resume here" (T8).
- Placeholders: none; the Help content is specified by section with its sources (README sections named), and every command is given.
- Type consistency: `router()`, `port_from_env`, `pages::{all, shell, render_markdown, with_heading_ids, not_found, Page}`, `assets::{ASSETS, Asset}`, `diagrams::{ALL, inline}` are used with the same names throughout.
- Review Focus: duplicate anchors (T2 test), trailing slash and case (T2 test), PORT fallback (T1 test), HEAD/healthz (T1, T7 tests), reduced motion (T5 test).
