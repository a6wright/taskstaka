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
