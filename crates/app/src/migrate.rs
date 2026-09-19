use std::env;

use anyhow::{Result, anyhow};

/// `fukulow migrate`: the only production path that reads the migrator's URL.
pub(crate) async fn run() -> Result<()> {
    let url = env::var("MIGRATOR_DATABASE_URL")
        .map_err(|_| anyhow!("MIGRATOR_DATABASE_URL is required and must be valid Unicode"))?;
    db::migrate(&url).await?;
    tracing::info!("Migrations are up to date");
    Ok(())
}
