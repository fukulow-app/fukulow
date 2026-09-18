mod config;
mod http;
mod invites;
mod sessions;
mod shutdown;
mod telemetry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init()?;
    let address = config::bind_address()?;
    let origin = config::public_origin()?;
    let pool = db::connect(&config::database_url()?).await?;
    let result = http::run(
        address,
        http::routes(sessions::StateData::new(pool.clone(), origin)),
    )
    .await;
    pool.close().await;
    result
}
