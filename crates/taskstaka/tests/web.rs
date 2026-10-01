//! The web shell against the real router: every page route serves the
//! shell, the assets are served with the right types, and the JSON API the
//! script relies on answers from the same origin.

use std::time::Duration;

use taskstaka::server::Bound;
use taskstaka_core::Store;
use ureq::Agent;

async fn start() -> (
    tempfile::TempDir,
    String,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::init(dir.path()).unwrap());
    let bound = Bound::bind(dir.path(), 0).await.unwrap();
    let addr = bound.addr.to_string();
    let handle = tokio::spawn(bound.serve());
    tokio::time::sleep(Duration::from_millis(100)).await;
    (dir, addr, handle)
}

fn agent() -> Agent {
    Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

fn get(agent: &Agent, url: &str) -> (u16, String, String) {
    let mut res = agent.get(url).call().unwrap();
    let status = res.status().as_u16();
    let ctype = res
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    (status, ctype, res.body_mut().read_to_string().unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_assets_and_api_from_one_origin() {
    let (_dir, addr, handle) = start().await;
    let a = agent();
    let base = format!("http://{addr}");

    for path in [
        "/",
        "/next",
        "/waiting",
        "/someday",
        "/done",
        "/t/abc123",
        "/?context=work",
    ] {
        let (status, ctype, html) = get(&a, &format!("{base}{path}"));
        assert_eq!(status, 200, "{path}");
        assert!(ctype.starts_with("text/html"), "{path}: {ctype}");
        assert!(
            html.contains(r#"<div id="app">"#) && html.contains("/app.js"),
            "{path}"
        );
    }
    let (status, ctype, js) = get(&a, &format!("{base}/app.js"));
    assert_eq!(status, 200);
    assert!(ctype.starts_with("application/javascript"), "{ctype}");
    assert!(js.contains("/api/v1/changes"), "live updates wired");
    let (status, ctype, css) = get(&a, &format!("{base}/app.css"));
    assert_eq!(status, 200);
    assert!(ctype.starts_with("text/css"), "{ctype}");
    assert!(css.contains("#pane"));

    // The script's first paint needs these to answer; a form-encoded
    // mutation must not (JSON only, ADR 0003).
    for path in [
        "/api/v1/tree?include_closed=true",
        "/api/v1/next?limit=0",
        "/api/v1/waiting",
        "/api/v1/nudges",
        "/api/v1/tasks?include_closed=true",
    ] {
        let (status, ctype, _) = get(&a, &format!("{base}{path}"));
        assert_eq!(status, 200, "{path}");
        assert!(ctype.starts_with("application/json"), "{path}: {ctype}");
    }
    let res = a
        .post(format!("{base}/api/v1/tasks"))
        .send_form([("title", "evil")])
        .unwrap();
    assert_eq!(res.status().as_u16(), 415, "form bodies are refused");

    // No forms remain.
    let res = a
        .post(format!("{base}/w/add"))
        .send_form([("title", "x")])
        .unwrap();
    assert_eq!(res.status().as_u16(), 404);

    taskstaka::client::Client::at(&addr).shutdown().unwrap();
    handle.await.unwrap().unwrap();
}
