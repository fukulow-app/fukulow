mod config;
mod http;
mod shutdown;
mod telemetry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init()?;
    http::run(config::bind_address()?, http::routes()).await
}
