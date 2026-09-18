//! Owns authentication, sessions, and invite tokens. It must never store or
//! log tokens in plaintext, so persistence and diagnostics cannot reveal them.

use argon2::{
    Argon2, PasswordHasher, PasswordVerifier, password_hash, password_hash::phc::PasswordHash,
};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
#[error("password hashing failed")]
pub struct PasswordHashError;

#[derive(Debug, thiserror::Error)]
#[error("token generation failed")]
pub struct TokenError;

/// An opaque browser credential; deliberately has no diagnostic representation.
pub struct SessionToken(String);

impl SessionToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn hash_password(password: &str) -> Result<String, PasswordHashError> {
    hash_password_with_rng(password, getrandom::fill)
}

fn hash_password_with_rng(
    password: &str,
    fill: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>,
) -> Result<String, PasswordHashError> {
    let mut salt = [0; 16];
    fill(&mut salt).map_err(|_| PasswordHashError)?;
    Argon2::default()
        .hash_password_with_salt(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordHashError)
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    verify_with(password, phc, |password, hash| {
        Argon2::default().verify_password(password.as_bytes(), hash)
    })
}

fn verify_with(
    password: &str,
    phc: &str,
    verify: impl Fn(&str, &PasswordHash) -> Result<(), password_hash::Error>,
) -> bool {
    // Only a mismatch is decided by the stored hash itself. Every other outcome — text
    // that does not parse, an unknown algorithm, parameters or a salt argon2 refuses —
    // is answered by verifying the dummy, so a stored hash that cannot be used costs
    // what a wrong password costs.
    //
    // Deciding that from the parsed fields instead would repeat argon2's own acceptance
    // rules here and get them subtly wrong: a salt of eight encoded characters passes a
    // length check on the text and decodes to six bytes, which argon2 rejects before
    // doing any work.
    let decided = PasswordHash::new(phc)
        .map_err(|_| ())
        // A hash with no salt or no output cannot be verified, but the verifier reports it
        // as a mismatch, so those two are checked here. Both are structure, not argon2's
        // acceptance rules.
        .and_then(|hash| match (&hash.salt, &hash.hash) {
            (Some(_), Some(_)) => Ok(hash),
            _ => Err(()),
        })
        .and_then(|hash| match verify(password, &hash) {
            Ok(()) => Ok(true),
            Err(password_hash::Error::PasswordInvalid) => Ok(false),
            Err(_) => Err(()),
        });
    decided.unwrap_or_else(|()| {
        if let Ok(dummy) = PasswordHash::new(db::DUMMY_PASSWORD_HASH) {
            let _ = verify(password, &dummy);
        }
        false
    })
}

pub fn new_session_token() -> Result<(SessionToken, db::TokenHash), TokenError> {
    let token = random_token(getrandom::fill)?;
    let hash = hash_session_token(&token);
    Ok((SessionToken(token), hash))
}

pub fn hash_session_token(token: &str) -> db::TokenHash {
    hash_token(token)
}

/// An invite credential has no diagnostic representation, like a session token.
pub struct InviteToken(String);

impl InviteToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn new_invite_token() -> Result<(InviteToken, db::TokenHash), TokenError> {
    let token = random_token(getrandom::fill)?;
    let hash = hash_invite_token(&token);
    Ok((InviteToken(token), hash))
}

pub fn hash_invite_token(token: &str) -> db::TokenHash {
    hash_token(token)
}

pub(crate) fn random_token(
    fill: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>,
) -> Result<String, TokenError> {
    let mut bytes = [0; 32];
    fill(&mut bytes).map_err(|_| TokenError)?;
    Ok(hex(&bytes))
}

pub(crate) fn hash_token(token: &str) -> db::TokenHash {
    db::TokenHash::from_bytes(Sha256::digest(token.as_bytes()).into())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                DIGITS[(byte >> 4) as usize] as char,
                DIGITS[(byte & 15) as usize] as char,
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests;
