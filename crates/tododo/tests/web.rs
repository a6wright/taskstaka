//! The HTML UI against the real router: pages render, forms mutate, and
//! cross-site form posts are refused.

use std::time::Duration;

use tododo::server::Bound;
use tododo_core::Store;
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
        .max_redirects(0)
        .build()
        .into()
}

fn get(agent: &Agent, url: &str) -> (u16, String) {
    let mut res = agent.get(url).call().unwrap();
    let status = res.status().as_u16();
    (status, res.body_mut().read_to_string().unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pages_render_and_forms_mutate() {
    let (_dir, addr, handle) = start().await;
    let a = agent();
    let base = format!("http://{addr}");

    let (status, html) = get(&a, &format!("{base}/"));
    assert_eq!(status, 200);
    assert!(html.contains("nothing open"), "{html}");
    let (status, css) = get(&a, &format!("{base}/style.css"));
    assert_eq!(status, 200);
    assert!(css.contains("--accent"));

    // Add via form (same-origin headers as a browser would send).
    let res = a
        .post(format!("{base}/w/add"))
        .header("Sec-Fetch-Site", "same-origin")
        .header("Origin", &base)
        .send_form([
            ("title", "Ship <auth> story"),
            ("context", "work"),
            ("back", "/"),
        ])
        .unwrap();
    assert_eq!(res.status().as_u16(), 303);
    let loc = res.headers()["location"].to_str().unwrap().to_string();
    assert!(loc.starts_with("/?msg=added+"), "{loc}");
    let id = loc.trim_start_matches("/?msg=added+").to_string();

    let (_, html) = get(&a, &format!("{base}{loc}"));
    assert!(
        html.contains("Ship &lt;auth&gt; story"),
        "escaped title:\n{html}"
    );
    assert!(html.contains(&format!(r#"href="/t/{id}""#)));

    // Subtask via the task page form, then the parent is blocked.
    let res = a
        .post(format!("{base}/w/add"))
        .header("Sec-Fetch-Site", "same-origin")
        .send_form([
            ("title", "Fix JWT"),
            ("parent", id.as_str()),
            ("back", &format!("/t/{id}")),
        ])
        .unwrap();
    assert_eq!(res.status().as_u16(), 303);
    let (_, html) = get(&a, &format!("{base}/t/{id}"));
    assert!(html.contains("blocked by 1"), "{html}");
    assert!(html.contains("Fix JWT"));

    // Closing the parent is refused with the store's message as a flash.
    let res = a
        .post(format!("{base}/w/{id}/status"))
        .header("Sec-Fetch-Site", "same-origin")
        .send_form([("status", "done"), ("back", &format!("/t/{id}"))])
        .unwrap();
    let loc = res.headers()["location"].to_str().unwrap().to_string();
    assert!(loc.contains("open+descendant"), "{loc}");

    // Edit form: title + due + tags.
    let res = a
        .post(format!("{base}/w/{id}/edit"))
        .header("Sec-Fetch-Site", "same-origin")
        .send_form([
            ("title", "Ship auth story"),
            ("context", "work"),
            ("priority", "high"),
            ("due", "+2d"),
            ("tags", "auth, backend"),
            ("parent", ""),
            ("body", "line one\r\nline two"),
            ("back", &format!("/t/{id}")),
        ])
        .unwrap();
    assert_eq!(res.status().as_u16(), 303);
    let (_, html) = get(&a, &format!("{base}/t/{id}"));
    assert!(
        html.contains("priority high") && html.contains("auth, backend"),
        "{html}"
    );
    assert!(html.contains("line one\nline two"));

    // Cross-site form post is refused.
    let res = a
        .post(format!("{base}/w/add"))
        .header("Sec-Fetch-Site", "cross-site")
        .send_form([("title", "evil")])
        .unwrap();
    assert_eq!(res.status().as_u16(), 403);
    let res = a
        .post(format!("{base}/w/add"))
        .header("Origin", "http://evil.example")
        .send_form([("title", "evil")])
        .unwrap();
    assert_eq!(res.status().as_u16(), 403);

    // Unknown task → JSON 404 from the shared error type.
    let (status, _) = get(&a, &format!("{base}/t/zzzzzz"));
    assert_eq!(status, 404);

    tododo::client::Client::at(&addr).shutdown().unwrap();
    handle.await.unwrap().unwrap();
}
