use super::*;
use argon2::{Algorithm, Params, Version};
use std::cell::Cell;

// Public, deliberately known test inputs, never credentials for an account. gitleaks:allow
const PASSWORD: &str = "fixture-password";
// This matches the fixed dummy hash so returning its result would authenticate. gitleaks:allow
const DUMMY_PASSWORD: &str = "fixture-dummy-password";

#[test]
fn passwords_use_argon2id_defaults_and_fresh_salts() {
    let first = hash_password(PASSWORD).unwrap();
    let second = hash_password(PASSWORD).unwrap();
    assert!(first.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    assert!(second.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    assert_ne!(first, second);
    assert!(verify_password(PASSWORD, &first));
    assert!(!verify_password("wrong", &first));
}

#[test]
fn lower_cost_stored_hashes_still_verify() {
    let params = Params::new(1024, 1, 1, None).unwrap();
    let hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_with_salt(PASSWORD.as_bytes(), b"fixture-salt")
        .unwrap()
        .to_string();
    assert!(verify_password(PASSWORD, &hash));
    assert!(!verify_password("wrong", &hash));
}

#[test]
fn malformed_hash_runs_one_dummy_verification_and_discards_success() {
    assert!(verify_password(DUMMY_PASSWORD, db::DUMMY_PASSWORD_HASH));
    for malformed in [
        "",
        "malformed",
        "$argon2id$v=19$m=19456,t=2,p=1",
        "$unknown$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$aGFzaA",
        // Parses, and argon2 refuses it: eight encoded characters decode to six salt
        // bytes. Judging the salt by the length of its text would take the fast path.
        "$argon2id$v=19$m=19456,t=2,p=1$YWJjZGVm$aGFzaA",
        // Parses, and argon2 refuses the parameters.
        "$argon2id$v=19$m=1,t=1,p=1$c29tZXNhbHQ$aGFzaA",
    ] {
        let dummy_verifications = Cell::new(0);
        let result = verify_with(DUMMY_PASSWORD, malformed, |password, hash| {
            if hash.to_string() == db::DUMMY_PASSWORD_HASH {
                dummy_verifications.set(dummy_verifications.get() + 1);
            }
            Argon2::default().verify_password(password.as_bytes(), hash)
        });
        assert!(!result, "{malformed}");
        // The dummy's own password must not authenticate, and must still cost its verification.
        assert_eq!(dummy_verifications.get(), 1, "{malformed}");
        assert!(!verify_password(DUMMY_PASSWORD, malformed));
    }
}

#[test]
fn rng_errors_are_opaque_at_password_and_token_boundaries() {
    let error =
        hash_password_with_rng(PASSWORD, |_| Err(getrandom::Error::UNSUPPORTED)).unwrap_err();
    assert_eq!(error.to_string(), "password hashing failed");
    assert!(!format!("{error:?}").contains(PASSWORD));
    assert_eq!(
        random_token(|_| Err(getrandom::Error::UNSUPPORTED))
            .unwrap_err()
            .to_string(),
        "token generation failed"
    );
}

#[test]
fn tokens_use_all_32_random_bytes_and_sha256_lowercase_hex() {
    let token = random_token(|bytes| {
        assert_eq!(bytes.len(), 32);
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = i as u8;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        token,
        (0..32).map(|i| format!("{i:02x}")).collect::<String>()
    );
    // The standard SHA-256 empty-input test vector, not a credential. gitleaks:allow
    const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    assert_eq!(hash_session_token("").as_str(), EMPTY_DIGEST);
    let (first, hash) = new_session_token().unwrap();
    let first = first.as_str().to_owned();
    let (second, _) = new_session_token().unwrap();
    assert_ne!(first, second.as_str());
    assert_eq!(first.len(), 64);
    assert!(
        first
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    );
    assert_eq!(hash.as_str(), hash_session_token(&first).as_str());
    assert_ne!(first, hash.as_str());
}

#[test]
fn invite_tokens_reuse_the_random_and_hash_steps_without_a_diagnostic_representation() {
    let (first, hash) = new_invite_token().unwrap();
    let (second, _) = new_invite_token().unwrap();
    assert_eq!(first.as_str().len(), 64);
    assert!(
        first
            .as_str()
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    assert_ne!(first.as_str(), second.as_str());
    assert_eq!(hash.as_str(), hash_token(first.as_str()).as_str());
    assert_eq!(
        hash_invite_token("abc").as_str(),
        hash_token("abc").as_str()
    );
    let source = include_str!("lib.rs");
    let function = source
        .split("pub fn new_invite_token()")
        .nth(1)
        .unwrap()
        .split("pub fn hash_invite_token")
        .next()
        .unwrap();
    assert!(function.contains("random_token(getrandom::fill)?"));
    assert!(!source.contains("impl std::fmt::Debug for InviteToken"));
    assert!(!source.contains("impl std::fmt::Display for InviteToken"));
}
