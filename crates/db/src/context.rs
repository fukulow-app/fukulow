use domain::ActorId;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};

/// A stored lookup key, never a plaintext session or invite token.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenHash(String);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("token hash must contain exactly 64 lowercase hexadecimal characters")]
pub struct InvalidTokenHash;

impl TokenHash {
    /// Encodes an already-computed digest; this type never hashes credentials.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        Self(
            bytes
                .iter()
                .flat_map(|byte| {
                    [
                        DIGITS[(byte >> 4) as usize] as char,
                        DIGITS[(byte & 15) as usize] as char,
                    ]
                })
                .collect(),
        )
    }

    pub fn from_hex(value: String) -> Result<Self, InvalidTokenHash> {
        if value.len() == 64
            && value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            Ok(Self(value))
        } else {
            Err(InvalidTokenHash)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) enum Context<'a> {
    Actor(ActorId),
    SessionToken(TokenHash),
    SignInEmail(&'a str),
    InviteToken(TokenHash),
}

pub(crate) async fn begin<'p>(
    pool: &'p PgPool,
    context: Context<'_>,
) -> Result<Transaction<'p, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    set_context(&mut tx, context).await?;
    Ok(tx)
}

// Invite acceptance creates its identity inside an existing transaction.
pub(crate) async fn set_context(
    conn: &mut PgConnection,
    context: Context<'_>,
) -> Result<(), sqlx::Error> {
    let (setting, value) = match context {
        Context::Actor(actor) => ("fukulow.actor_id", actor.0.to_string()),
        Context::SessionToken(hash) => ("fukulow.session_token_hash", hash.0),
        Context::SignInEmail(email) => ("fukulow.sign_in_email", email.to_owned()),
        Context::InviteToken(hash) => ("fukulow.invite_token_hash", hash.0),
    };
    sqlx::query!("SELECT set_config($1, $2, true)", setting, value)
        .fetch_one(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
