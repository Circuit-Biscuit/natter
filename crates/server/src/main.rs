use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("natter_server=info")),
        )
        .init();
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("natter-server {}", natter_server::VERSION);
        return Ok(());
    }
    natter_server::serve(natter_server::Config::from_env()?).await
}
