#[path = "../../db/tests/support/mod.rs"]
mod database;
#[allow(dead_code)]
#[path = "../src/http.rs"]
mod http;
#[path = "../src/invites.rs"]
mod invites;
#[allow(dead_code)]
mod session_support;
#[path = "../src/sessions.rs"]
mod sessions;
#[allow(dead_code)]
#[path = "../src/shutdown.rs"]
mod shutdown;

use database::{Database, Result, invites as fixtures};
use domain::{ActorId, InviteId, OrganizationId};
use serde_json::{Value, json};
use session_support::{EMAIL, ORIGIN, PASSWORD, Response, Server, assert_no_secrets, person};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

struct Fixture {
    db: Database,
    server: Arc<Server>,
    cookie: String,
    actor: ActorId,
    org: OrganizationId,
}
impl Fixture {
    async fn new(configure: impl FnOnce(&mut sessions::StateData)) -> Result<Self> {
        let db = Database::new().await?;
        let actor = person(&db).await?;
        let org = db.organization(actor).await?;
        let mut state = sessions::StateData::new(db.app.clone(), ORIGIN.into());
        configure(&mut state);
        let server = Arc::new(Server::new(state).await?);
        let cookie = server.sign_in().await?.cookie()?;
        Ok(Self {
            db,
            server,
            cookie,
            actor,
            org,
        })
    }
    fn path(&self) -> String {
        format!("/api/v1/organizations/{}/invites", self.org.0)
    }
    async fn request(&self, method: &str, path: &str, body: &str) -> Result<Response> {
        let response = self
            .server
            .request(method, path, Some(ORIGIN), Some(&self.cookie), body)
            .await?;
        assert_no_secrets(&[&self.cookie, EMAIL, PASSWORD]);
        Ok(response)
    }
    async fn create(&self, body: Value) -> Result<Value> {
        let response = self
            .request("POST", &self.path(), &body.to_string())
            .await?;
        assert_eq!(response.status, 201);
        let value: Value = serde_json::from_str(&response.body)?;
        assert_no_secrets(&[
            value["token"].as_str().unwrap(),
            value["link"].as_str().unwrap(),
        ]);
        Ok(value)
    }
    async fn token(&self, max_uses: i32) -> Result<(String, InviteId)> {
        let (token, hash) = auth::new_invite_token()?;
        let id = fixtures::invite(&self.db, self.actor, self.org, &hash, max_uses).await?;
        Ok((token.as_str().to_owned(), id))
    }
    async fn accept(&self, body: Value) -> Result<Response> {
        let response = self
            .request("POST", invites::ACCEPT_PATH, &body.to_string())
            .await?;
        for key in ["token", "email", "password"] {
            if let Some(value) = body[key].as_str().filter(|s| !s.is_empty()) {
                assert_no_secrets(&[value]);
            }
        }
        Ok(response)
    }
    async fn finish(self) -> Result {
        drop(self.server);
        self.db.finish().await
    }
}
fn valid(token: &str) -> Value {
    json!({"token":token,"email":"joiner@example.invalid","password":PASSWORD,"display_name":"  Fixture joiner\u{2003}"})
}
fn no_authority(response: &Response) {
    assert_eq!(response.status, 204);
    assert!(response.body.is_empty());
    assert!(
        !response
            .headers
            .iter()
            .any(|(key, _)| key == "set-cookie" || key == "authorization")
    );
}

#[tokio::test]
async fn owner_creates_accepts_and_revokes_with_exact_audits_and_secret_storage() -> Result {
    let f = Fixture::new(|_| {}).await?;
    let invite = f.create(json!({"expires_in":300,"max_uses":2})).await?;
    let token = invite["token"].as_str().unwrap();
    let id = InviteId(invite["id"].as_str().unwrap().parse()?);
    assert_eq!(id.0.get_version(), Some(uuid::Version::SortRand));
    assert_eq!(token.len(), 64);
    assert!(
        token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    assert_eq!(invite["link"], format!("{ORIGIN}/invite#{token}"));
    let row = fixtures::stored(&f.db, f.org, id).await?;
    assert_eq!(row["token_hash"], auth::hash_invite_token(token).as_str());
    assert!(!row.to_string().contains(token));
    no_authority(&f.accept(valid(token)).await?);
    let joined = fixtures::joined(&f.db, f.org, "joiner@example.invalid").await?;
    assert_eq!(joined["actor"]["type"], "human");
    assert_eq!(joined["actor"]["display_name"], "Fixture joiner");
    assert_eq!(joined["membership"]["status"], "active");
    assert_eq!(joined["membership"]["role"], "member");
    assert!(!joined.to_string().contains(PASSWORD));
    let phc = joined["user"]["password_hash"].as_str().unwrap().to_owned();
    assert!(tokio::task::spawn_blocking(move || auth::verify_password(PASSWORD, &phc)).await?);
    let audits = f.db.audit(f.org).await?;
    let created = audits
        .iter()
        .find(|e| e.action == "invite.created")
        .unwrap();
    assert_eq!(created.actor, f.actor.0);
    assert_eq!(created.target_type, "organization");
    assert_eq!(created.target, f.org.0);
    assert_eq!(
        created.metadata,
        json!({"invite_id":id.0,"kind":"organization","max_uses":2})
    );
    let added = audits
        .iter()
        .find(|e| e.metadata.get("invite_id").is_some() && e.action == "organization.member.added")
        .unwrap();
    assert_eq!(
        added.actor.to_string(),
        joined["actor"]["id"].as_str().unwrap()
    );
    assert_eq!(added.target_type, "actor");
    assert_eq!(added.target, added.actor);
    assert_eq!(added.metadata, json!({"role":"member","invite_id":id.0}));
    let path = format!("{}/{}", f.path(), id.0);
    no_authority(&f.request("DELETE", &path, "").await?);
    let before = f.db.audit(f.org).await?;
    no_authority(&f.request("DELETE", &path, "").await?);
    assert_eq!(f.db.audit(f.org).await?, before);
    let revoked = before
        .iter()
        .find(|e| e.action == "invite.revoked")
        .unwrap();
    assert_eq!(revoked.actor, f.actor.0);
    assert_eq!(revoked.target_type, "organization");
    assert_eq!(revoked.target, f.org.0);
    assert_eq!(revoked.metadata, json!({"invite_id":id.0}));
    f.accept(valid(token)).await?.assert_error(404);
    f.finish().await
}

#[tokio::test]
async fn creation_integer_types_bounds_default_and_utc_expiry() -> Result {
    let f = Fixture::new(|_| {}).await?;
    for (key, values) in [
        (
            "expires_in",
            vec![
                json!(299),
                json!(604801),
                json!(300.5),
                json!(300.0),
                json!("300"),
                Value::Null,
            ],
        ),
        (
            "max_uses",
            vec![
                json!(0),
                json!(101),
                json!(1.5),
                json!(1.0),
                json!("1"),
                Value::Null,
            ],
        ),
    ] {
        for value in values {
            let mut body = json!({"expires_in":300,"max_uses":1});
            body[key] = value;
            let before = fixtures::counts(&f.db).await?;
            f.request("POST", &f.path(), &body.to_string())
                .await?
                .assert_error(422);
            assert_eq!(fixtures::counts(&f.db).await?, before);
        }
    }
    for body in ["{}", "[]", "null", "42", "invalid"] {
        f.request("POST", &f.path(), body).await?.assert_error(422);
    }
    for (seconds, uses) in [(300, 1), (604800, 100)] {
        let before = OffsetDateTime::now_utc();
        let value = f
            .create(json!({"expires_in":seconds,"max_uses":uses}))
            .await?;
        assert_eq!(value["max_uses"], uses);
        let text = value["expires_at"].as_str().unwrap();
        assert!(text.ends_with('Z'));
        let expires = OffsetDateTime::parse(text, &Rfc3339)?;
        assert_eq!(expires.offset(), UtcOffset::UTC);
        assert!(expires >= before + time::Duration::seconds(seconds));
        assert!(expires <= OffsetDateTime::now_utc() + time::Duration::seconds(seconds));
    }
    assert_eq!(f.create(json!({"expires_in":300})).await?["max_uses"], 1);
    f.finish().await
}

#[tokio::test]
async fn origin_session_body_membership_capability_and_path_checks_are_ordered() -> Result {
    let f = Fixture::new(|_| {}).await?;
    let (token, id) = f.token(1).await?;
    let malformed = "/api/v1/organizations/not-a-uuid/invites";
    for path in [f.path(), malformed.into()] {
        for origin in [None, Some("https://foreign.example.invalid")] {
            f.server
                .request("POST", &path, origin, None, "invalid")
                .await?
                .assert_error(403);
            f.server
                .request("DELETE", &format!("{path}/bad"), origin, None, "")
                .await?
                .assert_error(403);
            f.server
                .request(
                    "POST",
                    invites::ACCEPT_PATH,
                    origin,
                    None,
                    &valid(&token).to_string(),
                )
                .await?
                .assert_error(403);
        }
        f.server
            .request("POST", &path, Some(ORIGIN), None, "invalid")
            .await?
            .assert_error(401);
        f.server
            .request("DELETE", &format!("{path}/bad"), Some(ORIGIN), None, "")
            .await?
            .assert_error(401);
        f.request("POST", &path, "invalid").await?.assert_error(422);
    }
    let before = fixtures::counts(&f.db).await?;
    f.request("POST", malformed, r#"{"expires_in":300}"#)
        .await?
        .assert_error(404);
    f.request("DELETE", &format!("{malformed}/{}", id.0), "")
        .await?
        .assert_error(404);
    f.request("DELETE", &format!("{}/bad", f.path()), "")
        .await?
        .assert_error(404);
    for id in [
        f.org.0.simple().to_string(),
        format!("{{{}}}", f.org.0),
        uuid::Uuid::now_v7().to_string(),
    ] {
        f.request(
            "POST",
            &format!("/api/v1/organizations/{id}/invites"),
            r#"{"expires_in":300}"#,
        )
        .await?
        .assert_error(404);
    }
    assert_eq!(fixtures::counts(&f.db).await?, before);
    assert_role_checks(&f).await?;
    let outsider = f.db.person().await?;
    let other = f.db.organization(outsider).await?;
    let path = format!("/api/v1/organizations/{}/invites", other.0);
    f.request("POST", &path, "invalid").await?.assert_error(422);
    f.request("POST", &path, r#"{"expires_in":300}"#)
        .await?
        .assert_error(404);
    f.request("DELETE", &format!("{path}/{}", id.0), "")
        .await?
        .assert_error(404);
    f.finish().await
}

async fn assert_role_checks(f: &Fixture) -> Result {
    for (role, status, expected) in [
        ("admin", "active", 201),
        ("member", "active", 403),
        ("owner", "suspended", 404),
        ("owner", "left", 404),
    ] {
        fixtures::set_role(&f.db, f.org, f.actor, role, status).await?;
        f.request("POST", &f.path(), "invalid")
            .await?
            .assert_error(422);
        let response = f
            .request("POST", &f.path(), r#"{"expires_in":300}"#)
            .await?;
        if expected == 201 {
            assert_eq!(response.status, expected);
            let value: Value = serde_json::from_str(&response.body)?;
            no_authority(
                &f.request(
                    "DELETE",
                    &format!("{}/{}", f.path(), value["id"].as_str().unwrap()),
                    "",
                )
                .await?,
            );
        } else {
            response.assert_error(expected);
        }
        f.request("DELETE", &format!("{}/bad", f.path()), "")
            .await?
            .assert_error(if expected == 201 { 404 } else { expected });
    }
    Ok(())
}

#[tokio::test]
async fn links_ignore_every_host_header_combination() -> Result {
    let f = Fixture::new(|_| {}).await?;
    let foreign = "foreign.example.invalid";
    for mask in [0, 1, 2, 4, 7] {
        let host = if mask & 1 == 0 { "localhost" } else { foreign };
        let forwarded_host = if mask & 2 == 0 {
            String::new()
        } else {
            format!("X-Forwarded-Host: {foreign}\r\n")
        };
        let forwarded = if mask & 4 == 0 {
            String::new()
        } else {
            format!("Forwarded: host={foreign};proto=https\r\n")
        };
        let body = r#"{"expires_in":300}"#;
        let response = f.server.raw(format!("POST {} HTTP/1.1\r\nHost: {host}\r\n{forwarded_host}{forwarded}Origin: {ORIGIN}\r\nCookie: {}={}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", f.path(), sessions::COOKIE_NAME, f.cookie, body.len())).await?;
        assert_eq!(response.status, 201);
        let value: Value = serde_json::from_str(&response.body)?;
        assert_eq!(
            value["link"],
            format!("{ORIGIN}/invite#{}", value["token"].as_str().unwrap())
        );
        assert_no_secrets(&[
            value["token"].as_str().unwrap(),
            value["link"].as_str().unwrap(),
        ]);
    }
    f.finish().await
}

#[tokio::test]
async fn revocation_is_scoped_even_for_an_owner_of_both_organizations() -> Result {
    let f = Fixture::new(|_| {}).await?;
    let other = f.db.organization(f.actor).await?;
    let (token, hash) = auth::new_invite_token()?;
    let id = fixtures::invite(&f.db, f.actor, other, &hash, 1).await?;
    let before = fixtures::stored(&f.db, other, id).await?;
    let audits_a = f.db.audit(f.org).await?;
    let audits_b = f.db.audit(other).await?;
    f.request("DELETE", &format!("{}/{}", f.path(), id.0), "")
        .await?
        .assert_error(404);
    assert_eq!(fixtures::stored(&f.db, other, id).await?, before);
    assert_eq!(f.db.audit(f.org).await?, audits_a);
    assert_eq!(f.db.audit(other).await?, audits_b);
    assert_no_secrets(&[token.as_str()]);
    f.finish().await
}

#[tokio::test]
async fn unusable_tokens_precede_all_other_fields_and_cost_no_hash() -> Result {
    let hashes = Arc::new(AtomicUsize::new(0));
    let count = hashes.clone();
    let f = Fixture::new(|state| {
        state.hash_invite_password = Arc::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Err(auth::PasswordHashError)
        })
    })
    .await?;
    for kind in ["unknown", "expired", "revoked", "exhausted", "team"] {
        let (token, hash) = auth::new_invite_token()?;
        let id = fixtures::unusable(&f.db, f.actor, f.org, &hash, kind).await?;
        let before = fixtures::counts(&f.db).await?;
        let old_row = match id {
            Some(id) => Some(fixtures::stored(&f.db, f.org, id).await?),
            None => None,
        };
        for body in [
            json!({"token":token.as_str()}),
            json!({"token":token.as_str(),"email":1}),
            json!({"token":token.as_str(),"email":"bad"}),
            valid(token.as_str()),
            json!({"token":token.as_str(),"email":"a\u{0}@b","display_name":"\u{0}"}),
        ] {
            f.accept(body).await?.assert_error(404);
        }
        assert_eq!(hashes.load(Ordering::SeqCst), 0);
        assert_eq!(fixtures::counts(&f.db).await?, before);
        if let Some(id) = id {
            assert_eq!(Some(fixtures::stored(&f.db, f.org, id).await?), old_row);
        }
    }
    f.finish().await
}

#[tokio::test]
async fn acceptance_body_shape_token_and_field_types_are_checked_in_order() -> Result {
    let f = Fixture::new(|_| {}).await?;
    for body in [
        "",
        "invalid",
        "null",
        "[]",
        "42",
        "\"text\"",
        "{}",
        "{\"token\":null}",
        "{\"token\":1}",
    ] {
        for content_type in ["application/json", "text/plain", ""] {
            f.server.raw(format!("POST {} HTTP/1.1\r\nHost: localhost\r\nOrigin: {ORIGIN}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", invites::ACCEPT_PATH, body.len())).await?.assert_error(422);
        }
    }
    let (token, id) = f.token(100).await?;
    let before = fixtures::counts(&f.db).await?;
    for field in ["email", "password", "display_name"] {
        let mut body = valid(&token);
        body.as_object_mut().unwrap().remove(field);
        f.accept(body).await?.assert_error(422);
        for value in [Value::Null, json!(1), json!([]), json!({}), json!(false)] {
            let mut body = valid(&token);
            body[field] = value;
            f.accept(body).await?.assert_error(422);
        }
    }
    assert_eq!(fixtures::counts(&f.db).await?, before);
    assert_eq!(fixtures::stored(&f.db, f.org, id).await?["used_count"], 0);
    f.finish().await
}

#[tokio::test]
async fn acceptance_bounds_use_bytes_scalars_and_trim_only_the_display_name() -> Result {
    let hashes = Arc::new(AtomicUsize::new(0));
    let count = hashes.clone();
    let f = Fixture::new(|state| {
        state.hash_invite_password = Arc::new(move |password| {
            count.fetch_add(1, Ordering::SeqCst);
            auth::hash_password(password)
        })
    })
    .await?;
    let (token, id) = f.token(100).await?;
    let invalid = [
        ("email", "@b".to_owned()),
        ("email", "a@@b".into()),
        ("email", "a b@c".into()),
        ("email", "a\u{2003}@b".into()),
        ("email", "a\0@b".into()),
        ("email", format!("{}a@b", "é".repeat(126))),
        ("password", "é".repeat(7)),
        ("password", format!("{}a", "é".repeat(512))),
        ("display_name", format!(" {} ", "é".repeat(81))),
        ("display_name", " \u{2003}\t\n".into()),
        ("display_name", "a\0b".into()),
    ];
    let before = fixtures::counts(&f.db).await?;
    for (field, value) in invalid {
        let mut body = valid(&token);
        body[field] = json!(value);
        f.accept(body).await?.assert_error(422);
    }
    assert_eq!(hashes.load(Ordering::SeqCst), 0);
    assert_eq!(fixtures::counts(&f.db).await?, before);
    assert_eq!(fixtures::stored(&f.db, f.org, id).await?["used_count"], 0);
    let cases = [
        ("email", "a@b".to_owned()),
        ("email", format!("{}@b", "é".repeat(126))),
        ("password", "é".repeat(8)),
        ("password", "é".repeat(512)),
        ("password", " \0abcdef ".into()),
        ("display_name", format!("\u{2003}{} ", "é".repeat(80))),
        ("display_name", " x ".into()),
    ];
    for (index, (field, value)) in cases.into_iter().enumerate() {
        let mut body = valid(&token);
        body["email"] = json!(format!("joiner-{index}@example.invalid"));
        body[field] = json!(value);
        no_authority(&f.accept(body.clone()).await?);
        let stored = fixtures::joined(&f.db, f.org, body["email"].as_str().unwrap()).await?;
        assert_eq!(
            stored["actor"]["display_name"],
            body["display_name"].as_str().unwrap().trim()
        );
        let password = body["password"].as_str().unwrap().to_owned();
        let phc = stored["user"]["password_hash"].as_str().unwrap().to_owned();
        assert!(tokio::task::spawn_blocking(move || auth::verify_password(&password, &phc)).await?);
    }
    f.finish().await
}

#[tokio::test]
async fn duplicate_email_rolls_back_the_actor_membership_audit_and_use() -> Result {
    let f = Fixture::new(|_| {}).await?;
    let (token, id) = f.token(1).await?;
    let before = fixtures::counts(&f.db).await?;
    let mut body = valid(&token);
    body["email"] = json!(EMAIL.to_ascii_uppercase());
    f.accept(body).await?.assert_error(409);
    assert_eq!(fixtures::counts(&f.db).await?, before);
    assert_eq!(fixtures::stored(&f.db, f.org, id).await?["used_count"], 0);
    no_authority(&f.accept(valid(&token)).await?);
    f.finish().await
}

#[tokio::test]
async fn rng_hashing_and_database_failures_are_empty_500_and_atomic() -> Result {
    for failure in ["rng", "hash", "create_db", "revoke_db", "accept_db"] {
        let f = Fixture::new(|state| {
            if failure == "rng" {
                state.new_invite_token = || Err(auth::TokenError);
            }
            if failure == "create_db" {
                state.new_invite_token = record_invite_token;
            }
            if failure == "hash" {
                state.hash_invite_password = Arc::new(|_| Err(auth::PasswordHashError));
            }
        })
        .await?;
        let (token, id) = f.token(1).await?;
        if failure.ends_with("_db") {
            fixtures::fail_audit(&f.db).await?;
        }
        let before = fixtures::counts(&f.db).await?;
        let row = fixtures::stored(&f.db, f.org, id).await?;
        match failure {
            "rng" | "create_db" => f
                .request("POST", &f.path(), r#"{"expires_in":300}"#)
                .await?
                .assert_error(500),
            "revoke_db" => f
                .request("DELETE", &format!("{}/{}", f.path(), id.0), "")
                .await?
                .assert_error(500),
            _ => f.accept(valid(&token)).await?.assert_error(500),
        }
        assert_eq!(fixtures::counts(&f.db).await?, before);
        assert_eq!(fixtures::stored(&f.db, f.org, id).await?, row);
        assert_no_secrets(&[&token, EMAIL, PASSWORD, &f.cookie]);
        if failure == "create_db" {
            let generated = GENERATED_INVITE.lock().unwrap().take().unwrap();
            assert_no_secrets(&[&generated, &format!("{ORIGIN}/invite#{generated}")]);
        }
        f.finish().await?;
    }
    Ok(())
}

#[tokio::test]
async fn precheck_is_only_a_filter_and_hashing_holds_no_transaction_or_invite_lock() -> Result {
    for revoke in [false, true] {
        let reached = Arc::new(tokio::sync::Notify::new());
        let (release, receiver) = std::sync::mpsc::channel();
        let receiver = std::sync::Mutex::new(receiver);
        let notify = reached.clone();
        let f = Fixture::new(|state| {
            state.hash_invite_password = Arc::new(move |_| {
                notify.notify_one();
                receiver
                    .lock()
                    .map_err(|_| auth::PasswordHashError)?
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .map_err(|_| auth::PasswordHashError)?;
                Ok(database::PASSWORD_HASH.into())
            })
        })
        .await?;
        let (token, id) = f.token(1).await?;
        let before = fixtures::counts(&f.db).await?;
        let server = f.server.clone();
        let body = valid(&token).to_string();
        let request = tokio::spawn(async move {
            server
                .request("POST", invites::ACCEPT_PATH, Some(ORIGIN), None, &body)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), reached.notified()).await?;
        fixtures::assert_no_app_transaction(&f.db).await?;
        if revoke {
            db::revoke_invite(&f.db.app, f.actor, f.org, Some(id)).await?;
        } else {
            fixtures::consume(&f.db.inspector, f.org, id).await?;
        }
        release.send(())?;
        request.await??.assert_error(404);
        let after = fixtures::counts(&f.db).await?;
        assert_eq!(
            (after.0, after.1, after.2, after.3),
            (before.0, before.1, before.2, before.3)
        );
        assert_eq!(after.4, before.4 + i64::from(revoke));
        assert_eq!(
            fixtures::stored(&f.db, f.org, id).await?["used_count"],
            if revoke { 0 } else { 1 }
        );
        assert_no_secrets(&[&token, "joiner@example.invalid", PASSWORD]);
        f.finish().await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twenty_concurrent_acceptances_admit_exactly_one_person() -> Result {
    let hashes = Arc::new(AtomicUsize::new(0));
    let count = hashes.clone();
    let gate = Arc::new((std::sync::Mutex::new(0), std::sync::Condvar::new()));
    let f = Fixture::new(|state| {
        state.hash_invite_password = Arc::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            let (arrivals, ready) = &*gate;
            let mut arrivals = arrivals.lock().map_err(|_| auth::PasswordHashError)?;
            *arrivals += 1;
            ready.notify_all();
            let (_arrivals, timeout) = ready
                .wait_timeout_while(arrivals, std::time::Duration::from_secs(10), |n| *n < 20)
                .map_err(|_| auth::PasswordHashError)?;
            if timeout.timed_out() {
                return Err(auth::PasswordHashError);
            }
            Ok(database::PASSWORD_HASH.into())
        })
    })
    .await?;
    let (token, id) = f.token(1).await?;
    let before = fixtures::counts(&f.db).await?;
    let mut requests = Vec::new();
    for index in 0..20 {
        let server = f.server.clone();
        let mut body = valid(&token);
        body["email"] = json!(format!("concurrent-{index}@example.invalid"));
        requests.push(tokio::spawn(async move {
            server
                .request(
                    "POST",
                    invites::ACCEPT_PATH,
                    Some(ORIGIN),
                    None,
                    &body.to_string(),
                )
                .await
        }));
    }
    let mut admitted = 0;
    for request in requests {
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(15), request).await???;
        if response.status == 204 {
            admitted += 1;
            no_authority(&response);
        } else {
            response.assert_error(404);
        }
    }
    assert_eq!(hashes.load(Ordering::SeqCst), 20);
    assert_eq!(admitted, 1);
    assert_eq!(
        fixtures::counts(&f.db).await?,
        (
            before.0 + 1,
            before.1 + 1,
            before.2 + 1,
            before.3,
            before.4 + 1
        )
    );
    assert_eq!(fixtures::stored(&f.db, f.org, id).await?["used_count"], 1);
    assert_no_secrets(&[&token, PASSWORD]);
    for index in 0..20 {
        assert_no_secrets(&[&format!("concurrent-{index}@example.invalid")]);
    }
    f.finish().await
}

#[tokio::test]
async fn registered_invite_routes_are_versioned_and_take_no_token_path_or_query() -> Result {
    let routes: Vec<_> = include_str!("../src/invites.rs")
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(".route("))
        .collect();
    assert_eq!(
        routes,
        [
            ".route(CREATE_PATH, post(create))",
            ".route(REVOKE_PATH, delete(revoke))",
            ".route(ACCEPT_PATH, post(accept))"
        ]
    );
    assert_eq!(
        [
            invites::CREATE_PATH,
            invites::REVOKE_PATH,
            invites::ACCEPT_PATH
        ],
        [
            "/api/v1/organizations/{organization_id}/invites",
            "/api/v1/organizations/{organization_id}/invites/{id}",
            "/api/v1/invite-acceptances",
        ]
    );
    let f = Fixture::new(|_| {}).await?;
    let (token, id) = f.token(1).await?;
    let before = fixtures::counts(&f.db).await?;
    for (method, path) in [
        ("POST", f.path()),
        ("DELETE", format!("{}/{}", f.path(), id.0)),
        ("POST", invites::ACCEPT_PATH.into()),
    ] {
        f.request(
            method,
            path.strip_prefix("/api/v1").unwrap(),
            &valid(&token).to_string(),
        )
        .await?
        .assert_error(404);
    }
    f.request("POST", &format!("{}/{}", invites::ACCEPT_PATH, token), "{}")
        .await?
        .assert_error(404);
    f.request(
        "POST",
        &format!("{}?token={token}", invites::ACCEPT_PATH),
        "{}",
    )
    .await?
    .assert_error(422);
    assert_eq!(fixtures::counts(&f.db).await?, before);
    f.finish().await
}

static GENERATED_INVITE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
fn record_invite_token() -> std::result::Result<(auth::InviteToken, db::TokenHash), auth::TokenError>
{
    let (token, hash) = auth::new_invite_token()?;
    *GENERATED_INVITE.lock().unwrap() = Some(token.as_str().to_owned());
    Ok((token, hash))
}

#[tokio::test]
async fn path_decoding_preserves_check_order_and_accepts_either_uuid_case() -> Result {
    let f = Fixture::new(|_| {}).await?;
    let malformed = "/api/v1/organizations/%FF/invites";
    f.server
        .request("POST", malformed, Some(ORIGIN), None, "invalid")
        .await?
        .assert_error(401);
    f.request("POST", malformed, "invalid")
        .await?
        .assert_error(422);
    f.request("POST", malformed, r#"{"expires_in":300}"#)
        .await?
        .assert_error(404);
    f.request("DELETE", &format!("{malformed}/bad"), "")
        .await?
        .assert_error(404);
    let path = format!(
        "/api/v1/organizations/{}/invites",
        f.org.0.to_string().to_ascii_uppercase().replace('-', "%2D")
    );
    let response = f.request("POST", &path, r#"{"expires_in":300}"#).await?;
    assert_eq!(response.status, 201);
    let value: Value = serde_json::from_str(&response.body)?;
    no_authority(
        &f.request(
            "DELETE",
            &format!(
                "{path}/{}",
                value["id"].as_str().unwrap().to_ascii_uppercase()
            ),
            "",
        )
        .await?,
    );
    f.request("DELETE", &format!("{path}/%FF"), "")
        .await?
        .assert_error(404);
    fixtures::set_role(&f.db, f.org, f.actor, "member", "active").await?;
    f.request("DELETE", &format!("{path}/%FF"), "")
        .await?
        .assert_error(403);
    assert_no_secrets(&[value["token"].as_str().unwrap()]);
    f.finish().await
}
