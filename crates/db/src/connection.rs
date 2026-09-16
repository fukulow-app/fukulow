use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, time::Duration};

#[derive(Debug, thiserror::Error)]
#[error("DATABASE_URL must name a reachable PostgreSQL database")]
pub struct ConnectError;

pub async fn connect(url: &str) -> Result<PgPool, ConnectError> {
    // Neither parsing errors nor PostgreSQL diagnostics may expose the URL or credentials.
    let options = PgConnectOptions::from_str(url)
        .map_err(|_| ConnectError)?
        .disable_statement_logging();
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(3))
        .connect_with(options)
        .await
        .map_err(|_| ConnectError)?;
    sqlx::query!("SELECT 1 AS alive")
        .fetch_one(&pool)
        .await
        .map_err(|_| ConnectError)?;
    Ok(pool)
}
