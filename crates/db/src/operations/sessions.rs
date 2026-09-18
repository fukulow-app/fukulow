use super::{SessionError, SignInError};
use crate::TokenHash;
use crate::context::{Context, begin, set_context};
use domain::{ActorId, ActorType};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

// A known password (fixture-dummy-password), never an identity. The verifier is called
// even when no user exists, and its result is discarded in that case. gitleaks:allow
pub const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$ZnVrdWxvdy1kdW1teS1zYWx0$OA3QfvZgfeDS0Ga5mZRi0S3nzxuaKJZEOshVa7Yv68s";

pub async fn sign_in(
    pool: &PgPool,
    email: &str,
    verify: impl Fn(&str) -> bool + Send + 'static,
    token_hash: &TokenHash,
    expires_at: OffsetDateTime,
) -> Result<ActorId, SignInError> {
    let mut tx = begin(pool, Context::SignInEmail(email)).await?;
    let person = sqlx::query!(r#"SELECT actor_id, password_hash FROM users WHERE lower(email COLLATE "C") = lower($1 COLLATE "C")"#, email)
        .fetch_optional(&mut *tx).await?;
    let phc = person
        .as_ref()
        .map_or(DUMMY_PASSWORD_HASH, |person| person.password_hash.as_str())
        .to_owned();
    // Argon2 must not occupy an async executor thread while the transaction is open.
    let verified = tokio::task::spawn_blocking(move || verify(&phc))
        .await
        .map_err(|_| SignInError::Database(sqlx::Error::WorkerCrashed))?;
    let person = person
        .filter(|_| verified)
        .ok_or(SignInError::InvalidCredentials)?;
    let actor = ActorId(person.actor_id);
    set_context(&mut tx, Context::Actor(actor)).await?;
    // Leaving the service may commit between the read above and the insert below, and the
    // insert would then fail on the deleted users row as a database error — a 500 where the
    // contract says 401. The share lock queues behind leave_service's FOR UPDATE on the
    // same actor, and the users row is read again under it: a departure that won the race
    // is seen, and one that lost it waits for this session and then removes it by cascade.
    // Taken after argon2, so a sign-in never holds up a departure for a whole verification.
    if super::lock_actor(&mut tx, actor, actor).await? != Some(false) {
        return Err(SignInError::InvalidCredentials);
    }
    let still_a_person = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM users WHERE actor_id = $1)",
        actor.0
    )
    .fetch_one(&mut *tx)
    .await?;
    if still_a_person != Some(true) {
        return Err(SignInError::InvalidCredentials);
    }
    sqlx::query!(
        "INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, $2, $3, $4)",
        Uuid::now_v7(),
        actor.0,
        token_hash.as_str(),
        expires_at
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(actor)
}

pub async fn resolve_session(
    pool: &PgPool,
    token_hash: &TokenHash,
) -> Result<ActorId, SessionError> {
    let mut tx = begin(pool, Context::SessionToken(token_hash.clone())).await?;
    let row = sqlx::query!("SELECT actor_id FROM sessions WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > now()", token_hash.as_str())
        .fetch_optional(&mut *tx).await?.ok_or(SessionError::NoSession)?;
    tx.commit().await?;
    Ok(ActorId(row.actor_id))
}

pub async fn revoke_session(
    pool: &PgPool,
    actor: ActorId,
    token_hash: &TokenHash,
) -> Result<(), SessionError> {
    let mut tx = begin(pool, Context::Actor(actor)).await?;
    let result = sqlx::query!("UPDATE sessions SET revoked_at = coalesce(revoked_at, now()) WHERE actor_id = $1 AND token_hash = $2", actor.0, token_hash.as_str())
        .execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(SessionError::NoSession);
    }
    tx.commit().await?;
    Ok(())
}

pub struct Me {
    pub actor_id: ActorId,
    pub actor_type: ActorType,
    pub display_name: Option<String>,
    pub email: Option<String>,
}

pub async fn me(pool: &PgPool, actor: ActorId) -> Result<Me, SessionError> {
    let mut tx = begin(pool, Context::Actor(actor)).await?;
    // RLS changes the prepared plan by role; sqlx cannot reliably infer LEFT JOIN nullability.
    let row = sqlx::query!(r#"SELECT a.id, a.type, a.display_name, u.email AS "email?" FROM actors a LEFT JOIN users u ON u.actor_id = a.id WHERE a.id = $1 AND a.deleted_at IS NULL"#, actor.0)
        .fetch_optional(&mut *tx).await?.ok_or(SessionError::NoSession)?;
    let actor_type = ActorType::parse(&row.r#type).ok_or(SessionError::Database(
        sqlx::Error::Decode("invalid actor type".into()),
    ))?;
    tx.commit().await?;
    Ok(Me {
        actor_id: ActorId(row.id),
        actor_type,
        display_name: row.display_name,
        email: row.email,
    })
}
