use clap::Parser;
use cmdline::Cmdline;
use config::Config;
use figment::providers::{Format, Toml};
use server::root;
use tracing_subscriber::{filter::EnvFilter, layer::SubscriberExt, util::SubscriberInitExt, Layer};

mod cmdline;
mod config;
mod server;

fn init_logging() {
    let console_subscriber = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_file(true)
        .with_thread_names(true)
        .with_line_number(true)
        .with_target(false)
        .with_ansi(true)
        .with_filter(EnvFilter::from_env("YADEX_LOGLEVEL"));
    tracing_subscriber::registry()
        .with(console_subscriber)
        .init();
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    init_logging();
    color_eyre::install()?;
    let cmdline = Cmdline::parse();
    tracing::info!("cmdline: {:?}", cmdline);
    let config: Config = figment::Figment::new()
        .merge(Toml::file(cmdline.config))
        .extract()?;
    let app = Router::new().route("/", get(root));

    let listener =
        tokio::net::TcpListener::bind((config.network.address, config.network.port)).await?;
    tracing::info!("Yadex listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
