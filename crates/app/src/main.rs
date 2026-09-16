mod config;
mod http;
mod shutdown;
mod telemetry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init()?;
    let address = config::bind_address()?;
    let pool = db::connect(&config::database_url()?).await?;
    let result = http::run(address, http::routes()).await;
    pool.close().await;
    result
}
