//! Owns authentication, sessions, and invite tokens. It must never store or
//! log tokens in plaintext, so persistence and diagnostics cannot reveal them.

use argon2::{
    Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version,
    password_hash::phc::PasswordHash,
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
        Argon2::default()
            .verify_password(password.as_bytes(), hash)
            .is_ok()
    })
}

fn verify_with(
    password: &str,
    phc: &str,
    verify: impl FnOnce(&str, &PasswordHash) -> bool,
) -> bool {
    let parsed = PasswordHash::new(phc).ok().filter(|hash| {
        Algorithm::try_from(hash.algorithm.as_str()).is_ok()
            && hash
                .version
                .is_none_or(|version| Version::try_from(version).is_ok())
            && Params::try_from(hash).is_ok()
            && hash.salt.as_ref().is_some_and(|salt| salt.len() >= 8)
            && hash.hash.is_some()
    });
    match parsed {
        Some(hash) => verify(password, &hash),
        None => {
            // Malformed stored hashes must cost a verification but can never authenticate.
            if let Ok(dummy) = PasswordHash::new(db::DUMMY_PASSWORD_HASH) {
                let _ = verify(password, &dummy);
            }
            false
        }
    }
}

pub fn new_session_token() -> Result<(SessionToken, db::TokenHash), TokenError> {
    let token = random_token(getrandom::fill)?;
    let hash = hash_session_token(&token);
    Ok((SessionToken(token), hash))
}

pub fn hash_session_token(token: &str) -> db::TokenHash {
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
