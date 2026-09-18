#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Clippy's test options do not cover integration-test helper functions"
)]

#[path = "../../db/tests/support/mod.rs"]
mod database;
#[expect(dead_code, reason = "Session tests do not run the production listener")]
#[path = "../src/http.rs"]
mod http;
#[path = "../src/invites.rs"]
mod invites;
#[path = "../src/route_registry.rs"]
mod route_registry;
mod session_support;
#[path = "../src/sessions.rs"]
mod sessions;
#[expect(dead_code, reason = "Session tests do not register shutdown signals")]
#[path = "../src/shutdown.rs"]
mod shutdown;

use axum_extra::extract::cookie::{Cookie, SameSite};
use database::{Database, Result};
use domain::{ActorId, ActorType};
use serde_json::json;
use session_support::{EMAIL, ORIGIN, PASSWORD, Server, assert_no_secrets, person};
use time::Duration;

#[tokio::test]
async fn sign_in_cookie_me_and_current_session_sign_out() -> Result {
    let db = Database::new().await?;
    database::sessions::clock_at_sign_in(&db).await?;
    let actor = person(&db).await?;
    let mut state = sessions::StateData::new(db.app.clone(), ORIGIN.into());
    state.now = database::sessions::fixed_now;
    let server = Server::new(state).await?;
    let first = server.sign_in().await?;
    assert_eq!(first.status, 204);
    assert!(first.body.is_empty());
    let token = first.cookie()?;
    assert_cookie(&first, 1_209_600)?;
    let hash = auth::hash_session_token(&token);
    let row = database::sessions::stored_session(&db, actor, &hash).await?;
    assert_eq!(row["token_hash"], hash.as_str());
    assert!(!row.to_string().contains(&token));
    assert_eq!(row["expires_at"], "2040-01-15T00:00:00+00:00");
    assert_eq!(row["created_at"], "2040-01-01T00:00:00+00:00");
    let response = server
        .request("GET", "/api/v1/me", None, Some(&token), "")
        .await?;
    assert_eq!(response.status, 200);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&response.body)?,
        json!({"actor":{"id":actor.0.to_string(),"type":"human","display_name":"Fixture person"},"user":{"email":EMAIL}})
    );
    database::sessions::clear_display_name(&db, actor).await?;
    let response = server
        .request("GET", "/api/v1/me", None, Some(&token), "")
        .await?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&response.body)?,
        json!({"actor":{"id":actor.0.to_string(),"type":"human","display_name":null},"user":{"email":EMAIL}})
    );
    let second = server.sign_in().await?.cookie()?;
    let signed_out = server
        .request(
            "DELETE",
            "/api/v1/sessions/current",
            Some(ORIGIN),
            Some(&token),
            "",
        )
        .await?;
    assert_eq!(signed_out.status, 204);
    assert_cookie(&signed_out, 0)?;
    assert_eq!(signed_out.cookie()?, "");
    server
        .request("GET", "/api/v1/me", None, Some(&token), "")
        .await?
        .assert_error(401);
    assert_eq!(
        server
            .request("GET", "/api/v1/me", None, Some(&second), "")
            .await?
            .status,
        200
    );
    assert_no_secrets(&[EMAIL, PASSWORD, &token, &second]);
    drop(server);
    db.finish().await
}

fn assert_cookie(response: &session_support::Response, max_age: i64) -> Result {
    let headers: Vec<_> = response
        .headers
        .iter()
        .filter(|(key, _)| key == "set-cookie")
        .collect();
    assert_eq!(headers.len(), 1);
    let cookie = Cookie::parse(headers[0].1.clone())?;
    assert_eq!(cookie.name(), "__Host-fukulow_session");
    assert_eq!(cookie.http_only(), Some(true));
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.same_site(), Some(SameSite::Strict));
    assert_eq!(cookie.path(), Some("/"));
    assert_eq!(cookie.max_age(), Some(Duration::seconds(max_age)));
    assert_eq!(cookie.domain(), None);
    assert_eq!(cookie.expires(), None);
    Ok(())
}

#[tokio::test]
async fn origin_precedes_credentials_body_and_every_mutating_handler() -> Result {
    let db = Database::new().await?;
    let actor = person(&db).await?;
    let server = Server::new(sessions::StateData::new(db.app.clone(), ORIGIN.into())).await?;
    let token = server.sign_in().await?.cookie()?;
    for origin in [
        None,
        Some("https://foreign.example.invalid"),
        Some("null"),
        Some("https://chat.example.invalid/"),
    ] {
        for body in [
            "null",
            "{}",
            &json!({"email":EMAIL,"password":PASSWORD}).to_string(),
        ] {
            server
                .request("POST", "/api/v1/sessions", origin, Some(&token), body)
                .await?
                .assert_error(403);
        }
        server
            .request(
                "DELETE",
                "/api/v1/sessions/current",
                origin,
                Some(&token),
                "",
            )
            .await?
            .assert_error(403);
        server
            .request(
                "DELETE",
                "/api/v1/sessions/current",
                origin,
                None,
                "invalid",
            )
            .await?
            .assert_error(403);
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            server
                .request(method, "/api/v1/me", origin, None, "{}")
                .await?
                .assert_error(403);
        }
    }
    for cookie in [None, Some("unknown")] {
        server
            .request(
                "DELETE",
                "/api/v1/sessions/current",
                Some(ORIGIN),
                cookie,
                "invalid",
            )
            .await?
            .assert_error(401);
    }
    server.raw(format!("POST /api/v1/sessions HTTP/1.1\r\nHost: localhost\r\nOrigin: {ORIGIN}\r\nOrigin: {ORIGIN}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}")).await?.assert_error(403);
    server.raw("GET /api/v1/me HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: close\r\n\r\n".into()).await?.assert_error(403);
    assert_eq!(database::sessions::session_count(&db, actor).await?, 1);
    assert_eq!(
        server
            .request("GET", "/api/v1/me", None, Some(&token), "")
            .await?
            .status,
        200
    );
    assert_no_secrets(&[EMAIL, PASSWORD, &token]);
    drop(server);
    db.finish().await
}

#[tokio::test]
async fn invalid_bodies_and_credentials_have_empty_indistinguishable_responses() -> Result {
    let db = Database::new().await?;
    let actor = person(&db).await?;
    let server = Server::new(sessions::StateData::new(db.app.clone(), ORIGIN.into())).await?;
    for body in [
        "",
        "invalid",
        "null",
        "[]",
        "42",
        "\"text\"",
        "{}",
        r#"{"email":"fixture@example.invalid"}"#,
        r#"{"password":"wrong"}"#,
        r#"{"email":0,"password":"wrong"}"#,
        r#"{"email":"fixture@example.invalid","password":null}"#,
        // U+0000 cannot reach PostgreSQL text: a malformed body, not a database error.
        r#"{"email":"fixture\u0000@example.invalid","password":"wrong"}"#,
        r#"{"email":"\u0000","password":"wrong"}"#,
    ] {
        for content_type in ["application/json", "text/plain", ""] {
            let header = if content_type.is_empty() {
                String::new()
            } else {
                format!("Content-Type: {content_type}\r\n")
            };
            server.raw(format!("POST /api/v1/sessions HTTP/1.1\r\nHost: localhost\r\nOrigin: {ORIGIN}\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())).await?.assert_error(422);
        }
    }
    for (email, password) in [
        (EMAIL, "wrong"),
        ("unknown@example.invalid", PASSWORD),
        ("unknown@example.invalid", session_support::DUMMY_PASSWORD),
    ] {
        server
            .request(
                "POST",
                "/api/v1/sessions",
                Some(ORIGIN),
                None,
                &json!({"email":email,"password":password}).to_string(),
            )
            .await?
            .assert_error(401);
    }
    assert_eq!(database::sessions::session_count(&db, actor).await?, 0);
    assert_no_secrets(&[
        EMAIL,
        PASSWORD,
        "unknown@example.invalid",
        session_support::DUMMY_PASSWORD,
    ]);
    drop(server);
    db.finish().await
}

#[tokio::test]
async fn missing_unknown_expired_and_revoked_sessions_all_return_empty_401() -> Result {
    let db = Database::new().await?;
    database::sessions::clock_at_sign_in(&db).await?;
    let actor = person(&db).await?;
    let mut state = sessions::StateData::new(db.app.clone(), ORIGIN.into());
    state.now = database::sessions::fixed_now;
    let server = Server::new(state).await?;
    let token = server.sign_in().await?.cookie()?;
    for cookie in [None, Some("unknown")] {
        server
            .request("GET", "/api/v1/me", None, cookie, "")
            .await?
            .assert_error(401);
    }
    server
        .request("GET", &format!("/api/v1/me?token={token}"), None, None, "")
        .await?
        .assert_error(401);
    server
        .request("GET", &format!("/api/v1/me/{token}"), None, None, "")
        .await?
        .assert_error(404);
    server.raw(format!("GET /api/v1/me HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n")).await?.assert_error(401);
    database::sessions::clock_before_expiry(&db).await?;
    assert_eq!(
        server
            .request("GET", "/api/v1/me", None, Some(&token), "")
            .await?
            .status,
        200
    );
    database::sessions::clock_after_expiry(&db).await?;
    server
        .request("GET", "/api/v1/me", None, Some(&token), "")
        .await?
        .assert_error(401);
    assert_eq!(
        database::sessions::stored_session(&db, actor, &auth::hash_session_token(&token)).await?["expires_at"],
        "2040-01-15T00:00:00+00:00"
    );
    database::sessions::clock_at_sign_in(&db).await?;
    db::revoke_session(&db.app, actor, &auth::hash_session_token(&token)).await?;
    server
        .request("GET", "/api/v1/me", None, Some(&token), "")
        .await?
        .assert_error(401);
    db::leave_service(&db.app, actor).await?;
    server.sign_in().await?.assert_error(401);
    assert_no_secrets(&[EMAIL, PASSWORD, &token]);
    drop(server);
    db.finish().await
}

#[tokio::test]
async fn rng_and_database_failures_return_empty_500_without_inserting_or_logging_secrets() -> Result
{
    for rng_failure in [true, false] {
        let db = Database::new().await?;
        let actor = person(&db).await?;
        let mut state = sessions::StateData::new(db.app.clone(), ORIGIN.into());
        if rng_failure {
            state.new_token = || Err(auth::TokenError);
        } else {
            database::sessions::fail_session_insert(&db).await?;
            state.new_token = record_token;
        }
        let server = Server::new(state).await?;
        server.sign_in().await?.assert_error(500);
        assert_eq!(database::sessions::session_count(&db, actor).await?, 0);
        let token = GENERATED_TOKEN
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| "no-token-generated".into());
        assert_no_secrets(&[EMAIL, PASSWORD, &token]);
        drop(server);
        db.finish().await?;
    }
    Ok(())
}

#[test]
fn nonhuman_json_omits_user_and_preserves_each_canonical_type() {
    for actor_type in [ActorType::Bot, ActorType::Integration, ActorType::System] {
        let actor = ActorId(uuid::Uuid::now_v7());
        let me = db::Me {
            actor_id: actor,
            actor_type,
            display_name: None,
            email: None,
        };
        assert_eq!(
            sessions::me_json(me),
            json!({"actor":{"id":actor.0.to_string(),"type":actor_type.as_str(),"display_name":null}})
        );
    }
}

static GENERATED_TOKEN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

static TOKEN_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn failing_token() -> std::result::Result<(auth::SessionToken, db::TokenHash), auth::TokenError> {
    TOKEN_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Err(auth::TokenError)
}

/// The token is made only once the credential is accepted, so a failing RNG cannot turn a
/// credential failure into a 500, and a failed attempt asks the RNG for nothing.
#[tokio::test]
async fn credential_failures_are_401_even_when_the_rng_fails_and_spend_no_token() -> Result {
    use std::sync::atomic::Ordering::SeqCst;
    let db = Database::new().await?;
    let actor = person(&db).await?;
    let mut state = sessions::StateData::new(db.app.clone(), ORIGIN.into());
    state.new_token = failing_token;
    let server = Server::new(state).await?;
    TOKEN_CALLS.store(0, SeqCst);
    for (email, password) in [(EMAIL, "wrong"), ("unknown@example.invalid", PASSWORD)] {
        server
            .request(
                "POST",
                "/api/v1/sessions",
                Some(ORIGIN),
                None,
                &json!({"email":email,"password":password}).to_string(),
            )
            .await?
            .assert_error(401);
    }
    assert_eq!(TOKEN_CALLS.load(SeqCst), 0);
    // With the credential accepted, the RNG is asked once and its failure is the 500.
    server.sign_in().await?.assert_error(500);
    assert_eq!(TOKEN_CALLS.load(SeqCst), 1);
    assert_eq!(database::sessions::session_count(&db, actor).await?, 0);
    drop(server);
    db.finish().await
}

fn record_token() -> std::result::Result<(auth::SessionToken, db::TokenHash), auth::TokenError> {
    let (token, hash) = auth::new_session_token()?;
    *GENERATED_TOKEN.lock().unwrap() = Some(token.as_str().to_owned());
    Ok((token, hash))
}

#[tokio::test]
async fn every_registered_route_obeys_the_http_contract() -> Result {
    let db = Database::new().await?;
    person(&db).await?;
    let server = Server::new(sessions::StateData::new(db.app.clone(), ORIGIN.into())).await?;
    let unknown = auth::new_session_token()?.0;
    let mut failures = Vec::new();
    for route in route_registry::ROUTES {
        let path = route
            .path
            .split('/')
            .map(|part| {
                if part.starts_with('{') && part.ends_with('}') {
                    uuid::Uuid::now_v7().to_string()
                } else {
                    part.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        let mut rows = Vec::new();
        if route.changes_state() {
            for origin in [None, Some("https://foreign.example.invalid")] {
                rows.push((origin, "missing", path.as_str(), Some(403)));
                if route.needs_session {
                    rows.push((origin, "unknown", path.as_str(), Some(403)));
                    rows.push((origin, "valid", path.as_str(), Some(403)));
                }
            }
        }
        rows.push((
            Some(ORIGIN),
            "missing",
            &path,
            route.needs_session.then_some(401),
        ));
        if route.needs_session {
            rows.push((Some(ORIGIN), "unknown", &path, Some(401)));
        }
        if route.path != "/health" {
            rows.push((
                Some(ORIGIN),
                "missing",
                path.strip_prefix("/api/v1")
                    .expect("API route must be versioned"),
                Some(404),
            ));
        }
        for (origin, credential, path, expected) in rows {
            // A broken Origin guard can revoke a session; each valid row gets a fresh
            // one so later rows still prove the order against a credential that works.
            let valid = if credential == "valid" {
                Some(server.sign_in().await?.cookie()?)
            } else {
                None
            };
            let token = match credential {
                "valid" => valid.as_deref(),
                "unknown" => Some(unknown.as_str()),
                _ => None,
            };
            let response = contract_request(&server, route, path, origin, token).await?;
            let status_matches =
                expected.map_or(response.status != 401, |status| response.status == status);
            let empty_error = response.status < 400
                || (response.body.is_empty()
                    && !response
                        .headers
                        .iter()
                        .any(|(name, _)| name == "set-cookie"));
            if !status_matches || !empty_error {
                failures.push(format!("{} {}: origin={origin:?}, credential={credential}, path={path}, expected={expected:?} (None means not 401), status={}, empty_body={}, no_set_cookie={}", route.method, route.path, response.status, response.body.is_empty(), !response.headers.iter().any(|(name, _)| name == "set-cookie")));
            }
        }
    }
    drop(server);
    db.finish().await?;
    assert!(
        failures.is_empty(),
        "HTTP contract violations:\n{}",
        failures.join("\n")
    );
    Ok(())
}

async fn contract_request(
    server: &Server,
    route: &route_registry::Route,
    path: &str,
    origin: Option<&str>,
    token: Option<&str>,
) -> Result<session_support::Response> {
    let origin_header = origin
        .map(|value| format!("Origin: {value}\r\n"))
        .unwrap_or_default();
    let cookie = token
        .map(|value| format!("Cookie: {}={value}\r\n", sessions::COOKIE_NAME))
        .unwrap_or_default();
    // RFC 6455 example nonce, never an authentication credential.
    const UPGRADE_HEADERS: &str = "Connection: close, Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"; // gitleaks:allow
    let upgrade = if route.websocket {
        UPGRADE_HEADERS
    } else {
        "Connection: close\r\n"
    };
    // Malformed JSON prevents a public route from creating anything. Protected
    // routes must reject the credential before they attempt to read this body.
    server.raw(format!("{} {path} HTTP/1.1\r\nHost: localhost\r\n{origin_header}{cookie}{upgrade}Content-Length: 1\r\n\r\n{{", route.method)).await
}
