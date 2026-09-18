use super::{Database, Result};
use domain::ActorId;
use time::OffsetDateTime;

pub(crate) fn fixed_now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(2_208_988_800).unwrap()
}

// Only the disposable database's clock is replaced; production still uses PostgreSQL now().
pub(crate) async fn clock_at_sign_in(db: &Database) -> Result {
    sqlx::query("CREATE OR REPLACE FUNCTION pg_catalog.now() RETURNS timestamptz LANGUAGE sql STABLE AS $$ SELECT '2040-01-01 00:00:00+00'::timestamptz $$").execute(&db.inspector).await?;
    Ok(())
}

pub(crate) async fn clock_before_expiry(db: &Database) -> Result {
    sqlx::query("CREATE OR REPLACE FUNCTION pg_catalog.now() RETURNS timestamptz LANGUAGE sql STABLE AS $$ SELECT '2040-01-14 23:59:59.999999+00'::timestamptz $$").execute(&db.inspector).await?;
    Ok(())
}

pub(crate) async fn clock_at_expiry(db: &Database) -> Result {
    sqlx::query("CREATE OR REPLACE FUNCTION pg_catalog.now() RETURNS timestamptz LANGUAGE sql STABLE AS $$ SELECT '2040-01-15 00:00:00+00'::timestamptz $$").execute(&db.inspector).await?;
    Ok(())
}

pub(crate) async fn clock_after_expiry(db: &Database) -> Result {
    sqlx::query("CREATE OR REPLACE FUNCTION pg_catalog.now() RETURNS timestamptz LANGUAGE sql STABLE AS $$ SELECT '2040-01-15 00:00:00.000001+00'::timestamptz $$").execute(&db.inspector).await?;
    Ok(())
}

pub(crate) async fn stored_session(
    db: &Database,
    actor: ActorId,
    hash: &db::TokenHash,
) -> Result<serde_json::Value> {
    let mut tx = db.actor(actor).await?;
    let row = sqlx::query_scalar(
        "SELECT to_jsonb(s) FROM sessions s WHERE actor_id = $1 AND token_hash = $2",
    )
    .bind(actor.0)
    .bind(hash.as_str())
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub(crate) async fn session_count(db: &Database, actor: ActorId) -> Result<i64> {
    let mut tx = db.actor(actor).await?;
    let count = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE actor_id = $1")
        .bind(actor.0)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(count)
}

pub(crate) async fn fail_session_insert(db: &Database) -> Result {
    sqlx::query("REVOKE INSERT ON sessions FROM fukulow_app")
        .execute(&db.owner)
        .await?;
    Ok(())
}

pub(crate) async fn clear_display_name(db: &Database, actor: ActorId) -> Result {
    let mut tx = db.actor(actor).await?;
    sqlx::query("UPDATE actors SET display_name = NULL WHERE id = $1")
        .bind(actor.0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
