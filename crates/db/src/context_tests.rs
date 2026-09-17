use super::*;
use sqlx::{
    Acquire, ConnectOptions,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::str::FromStr;
use uuid::Uuid;

type Result = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn pool() -> std::result::Result<PgPool, Box<dyn std::error::Error + Send + Sync>> {
    let options =
        PgConnectOptions::from_str(&std::env::var("DATABASE_URL")?)?.disable_statement_logging();
    Ok(PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}

#[tokio::test]
async fn actor_and_invite_contexts_do_not_survive_commit_or_rollback() -> Result {
    let pool = pool().await?;
    let actor = ActorId(Uuid::now_v7());
    for commit in [true, false] {
        let hash = "a".repeat(64);
        for context in [
            Context::Actor(actor),
            Context::InviteToken(TokenHash::from_hex(hash.clone())?),
        ] {
            let mut tx = begin(&pool, context).await?;
            let values: (Option<String>, Option<String>) = sqlx::query_as("SELECT nullif(current_setting('fukulow.actor_id', true), ''), nullif(current_setting('fukulow.invite_token_hash', true), '')").fetch_one(&mut *tx).await?;
            assert!(values.0 == Some(actor.0.to_string()) || values.1 == Some(hash.clone()));
            if commit {
                tx.commit().await?;
            } else {
                tx.rollback().await?;
            }
            let mut next = begin(&pool, Context::SignInEmail("fixture@example.invalid")).await?;
            let empty: bool = sqlx::query_scalar("SELECT nullif(current_setting('fukulow.actor_id', true), '') IS NULL AND nullif(current_setting('fukulow.invite_token_hash', true), '') IS NULL").fetch_one(&mut *next).await?;
            assert!(empty);
            next.commit().await?;
        }
    }
    pool.close().await;
    Ok(())
}

#[path = "../tests/support/mod.rs"]
mod support;

#[tokio::test]
async fn invite_context_creates_a_person_but_admits_no_membership_yet() -> Result {
    let db = support::Database::new().await?;
    let a = support::Conversation::new(&db).await?;
    let b = support::Conversation::new(&db).await?;
    let hash = TokenHash::from_hex("a".repeat(64))?;
    sqlx::query("INSERT INTO invites (id, organization_id, kind, token_hash, expires_at, max_uses, created_by_actor_id) VALUES ($1, $2, 'organization', $3, now() + interval '1 day', 1, $4)")
        .bind(Uuid::now_v7()).bind(a.organization.0).bind(hash.as_str()).bind(a.owner.0)
        .execute(&db.inspector).await?;
    let mut tx = begin(&db.app, Context::InviteToken(hash)).await?;
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT organization_id FROM invites")
            .fetch_all(&mut *tx)
            .await?,
        [a.organization.0]
    );
    let actor = crate::create_person(
        &mut tx,
        "invited@example.invalid",
        support::PASSWORD_HASH,
        "Fixture invitee",
    )
    .await?;
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT actor_id FROM users")
            .fetch_all(&mut *tx)
            .await?,
        [actor.0]
    );
    let mut attempt = tx.begin().await?;
    support::rejected(
        sqlx::query("INSERT INTO organization_members (organization_id, actor_id) VALUES ($1, $2)")
            .bind(b.organization.0)
            .bind(actor.0)
            .execute(&mut *attempt)
            .await,
        "42501",
    );
    attempt.rollback().await?;
    // Even the invite's own organization is refused: #3 adds the invite branch together with a
    // database-side tie to a successful use of the invite in the same transaction.
    let mut attempt = tx.begin().await?;
    support::rejected(
        sqlx::query("INSERT INTO organization_members (organization_id, actor_id) VALUES ($1, $2)")
            .bind(a.organization.0)
            .bind(actor.0)
            .execute(&mut *attempt)
            .await,
        "42501",
    );
    attempt.rollback().await?;
    tx.commit().await?;
    let memberships: i64 =
        sqlx::query_scalar("SELECT count(*) FROM organization_members WHERE actor_id = $1")
            .bind(actor.0)
            .fetch_one(&db.inspector)
            .await?;
    assert_eq!(memberships, 0);
    db.finish().await
}
