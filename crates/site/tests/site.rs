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
    assert!(
        ctype.starts_with("text/html") && body.contains("Not found"),
        "{body}"
    );
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
    assert_eq!(
        res.headers().get("location").unwrap().to_str().unwrap(),
        "/install"
    );
    let (status, _, _) = get(&base, "/INSTALL");
    assert_eq!(
        status, 404,
        "paths are case-sensitive; the links on the site are lowercase"
    );
    handle.abort();
}
