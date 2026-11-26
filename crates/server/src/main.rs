use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("laklak_server=info")),
        )
        .init();
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("laklak-server {}", laklak_server::VERSION);
        return Ok(());
    }
    laklak_server::serve(laklak_server::Config::from_env()?).await
}
