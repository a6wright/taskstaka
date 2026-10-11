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
