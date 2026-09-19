use sqlx::migrate::MigrateError as SqlxMigrateError;

use crate::connect;

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("MIGRATOR_DATABASE_URL must name a reachable PostgreSQL database")]
    Connect,
    // Only the version and SQLSTATE: PostgreSQL's message and detail can quote row
    // values, such as a constraint validated over existing personal data.
    #[error(
        "migration {} failed with SQLSTATE {}",
        .version.map_or_else(|| "(unknown)".to_owned(), |v| v.to_string()),
        .code.as_deref().unwrap_or("(none)")
    )]
    Migration {
        version: Option<i64>,
        code: Option<String>,
    },
}

/// Applies the migrations embedded at build time and returns. Only `fukulow migrate`
/// calls this; the server never migrates, so it never needs the migrator's credentials.
pub async fn migrate(url: &str) -> Result<(), MigrateError> {
    let pool = connect(url).await.map_err(|_| MigrateError::Connect)?;
    let result = sqlx::migrate!("../../migrations").run(&pool).await;
    pool.close().await;
    result.map_err(|error| {
        let (version, source) = match &error {
            SqlxMigrateError::ExecuteMigration(source, version) => (Some(*version), Some(source)),
            SqlxMigrateError::Execute(source) => (None, Some(source)),
            _ => (None, None),
        };
        MigrateError::Migration {
            version,
            code: source
                .and_then(|source| source.as_database_error())
                .and_then(|error| error.code())
                .map(|code| code.into_owned()),
        }
    })
}
