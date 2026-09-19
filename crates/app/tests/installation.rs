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
#[expect(
    dead_code,
    reason = "Bootstrap tests do not run the production listener"
)]
#[path = "../src/http.rs"]
mod http;
#[path = "../src/invites.rs"]
mod invites;
#[expect(
    dead_code,
    reason = "The session test binary runs the route contract sweep"
)]
#[path = "../src/route_registry.rs"]
mod route_registry;
#[expect(
    dead_code,
    reason = "Bootstrap tests use only part of the session test support"
)]
mod session_support;
#[path = "../src/sessions.rs"]
mod sessions;
#[expect(dead_code, reason = "Bootstrap tests do not register shutdown signals")]
#[path = "../src/shutdown.rs"]
mod shutdown;

#[expect(
    dead_code,
    reason = "Function tests exercise bootstrap without terminal input"
)]
#[path = "../src/bootstrap.rs"]
mod bootstrap;
#[expect(dead_code, reason = "Bootstrap does not use server configuration")]
#[path = "../src/config.rs"]
mod config;

use bootstrap::{Answers, Failure, execute};
use database::{Database, Result, installation as fixtures};
use serde_json::json;
use session_support::{EMAIL, ORIGIN, PASSWORD, Server, assert_no_secrets};
use std::sync::Arc;

fn answers() -> Answers {
    assert_no_secrets(&[EMAIL, PASSWORD]);
    Answers {
        organization_name: " \u{2003}Fixture organization \u{2003}".into(),
        slug: "fixture".into(),
        display_name: " \u{2003}Fixture owner \u{2003}".into(),
        email: EMAIL.into(),
        password: PASSWORD.into(),
        confirmation: PASSWORD.into(),
    }
}

fn assert_failure(error: &Failure, field: &str, values: &[&str]) {
    assert_eq!(error.code, 1);
    assert!(error.message.to_ascii_lowercase().contains(field));
    for value in values.iter().copied().filter(|v| !v.is_empty()) {
        assert!(!error.message.contains(value));
        assert!(!format!("{error:?}").contains(value));
    }
    assert_no_secrets(
        &values
            .iter()
            .copied()
            .filter(|value| value.len() >= 8)
            .collect::<Vec<_>>(),
    );
}

async fn invalid(field: &str, values: Vec<String>) -> Result {
    let db = Database::new().await?;
    let before = fixtures::counts(&db).await?;
    for value in values {
        let mut input = answers();
        match field {
            "email" => input.email = value.clone(),
            "password" => {
                input.password = value.clone();
                input.confirmation = value.clone();
            }
            "display name" => input.display_name = value.clone(),
            "organization name" => input.organization_name = value.clone(),
            "slug" => input.slug = value.clone(),
            _ => panic!("unknown fixture field"),
        }
        // Validation must precede even hashing; rollback alone could conceal an
        // invalid value that reached the database and was rejected there.
        let error =
            bootstrap::execute_with_hasher(&db.app, input, |_| Err(auth::PasswordHashError))
                .await
                .unwrap_err();
        assert_failure(&error, field, &[&value, EMAIL, PASSWORD]);
        assert_eq!(fixtures::counts(&db).await?, before);
    }
    db.finish().await
}

#[tokio::test]
async fn email_rejects_every_invalid_bound_and_format_before_writing() -> Result {
    invalid(
        "email",
        vec![
            "".into(),
            "a@".into(),
            format!("{}@b", "a".repeat(253)),
            "missing-at".into(),
            "a@@b".into(),
            " a@b".into(),
            "a@b ".into(),
            "a@\u{2003}b".into(),
        ],
    )
    .await
}

#[tokio::test]
async fn password_rejects_below_scalar_and_above_byte_bounds_before_writing() -> Result {
    invalid(
        "password",
        vec![
            "".into(),
            "界".repeat(7),
            "x".repeat(1025),
            "界".repeat(342),
        ],
    )
    .await
}

#[tokio::test]
async fn display_name_rejects_both_invalid_bounds_before_writing() -> Result {
    invalid(
        "display name",
        vec!["".into(), " \u{2003}\t".into(), "界".repeat(81)],
    )
    .await
}

#[tokio::test]
async fn organization_name_rejects_both_invalid_bounds_before_writing() -> Result {
    invalid(
        "organization name",
        vec!["".into(), " \u{2003}\t".into(), "界".repeat(81)],
    )
    .await
}

#[tokio::test]
async fn slug_rejects_invalid_bounds_and_format_before_writing() -> Result {
    invalid(
        "slug",
        [
            "",
            "-a",
            "a-",
            "UPPER",
            "a_b",
            "a/b",
            "é",
            "a\n",
            " a",
            "a ",
            &"a".repeat(33),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    )
    .await
}

#[tokio::test]
async fn email_refuses_nul_before_writing() -> Result {
    invalid("email", vec!["fixture\0@example.invalid".into()]).await
}

#[tokio::test]
async fn display_name_refuses_nul_before_writing() -> Result {
    invalid("display name", vec!["Fixture\0owner".into()]).await
}

#[tokio::test]
async fn organization_name_refuses_nul_before_writing() -> Result {
    invalid("organization name", vec!["Fixture\0organization".into()]).await
}

#[tokio::test]
async fn confirmation_must_match_without_trimming() -> Result {
    let db = Database::new().await?;
    let before = fixtures::counts(&db).await?;
    let mut input = answers();
    input.confirmation.push(' ');
    let error = execute(&db.app, input).await.unwrap_err();
    assert_failure(&error, "password", &[EMAIL, PASSWORD]);
    assert_eq!(fixtures::counts(&db).await?, before);
    db.finish().await
}

#[test]
fn exact_valid_bounds_use_bytes_and_unicode_scalars() {
    for (email, password, name, slug) in [
        ("a@b".to_owned(), "界".repeat(8), "界".into(), "a".into()),
        (
            format!("{}@b", "é".repeat(126)),
            "界".repeat(341) + "x",
            "界".repeat(80),
            "a".repeat(32),
        ),
        (
            "a@b".to_owned(),
            " \0fixture ".into(),
            "Fixture".into(),
            "a--0".into(),
        ),
    ] {
        let mut input = answers();
        input.email = email.clone();
        input.password = password.clone();
        input.confirmation = password.clone();
        input.display_name = format!(" \u{2003}{name}\t");
        input.organization_name = format!(" \u{2003}{name}\t");
        input.slug = slug.clone();
        input.validate().unwrap();
        assert_eq!(input.email, email);
        assert_eq!(input.password, password);
        assert_eq!(input.display_name, name);
        assert_eq!(input.organization_name, name);
        assert_eq!(input.slug, slug);
    }
}

#[tokio::test]
async fn success_stores_trimmed_identity_owner_and_exact_audit_records() -> Result {
    let db = Database::new().await?;
    let org = execute(&db.app, answers()).await.unwrap();
    let stored = fixtures::stored(&db, org).await?;
    let actor = &stored["actor"]["id"];
    assert_eq!(stored["organization"]["name"], "Fixture organization");
    assert_eq!(stored["organization"]["slug"], "fixture");
    assert_eq!(stored["actor"]["display_name"], "Fixture owner");
    assert_eq!(stored["actor"]["type"], "human");
    assert_eq!(stored["user"]["actor_id"], *actor);
    assert_eq!(stored["user"]["email"], EMAIL);
    let hash = stored["user"]["password_hash"].as_str().unwrap().to_owned();
    assert!(tokio::task::spawn_blocking(move || auth::verify_password(PASSWORD, &hash)).await?);
    assert_eq!(stored["membership"]["organization_id"], org.0.to_string());
    assert_eq!(stored["membership"]["actor_id"], *actor);
    assert_eq!(stored["membership"]["role"], "owner");
    assert_eq!(stored["membership"]["status"], "active");
    assert_eq!(stored["installation"]["bootstrapped_by_actor_id"], *actor);
    assert_eq!(stored["installation"]["singleton"], true);
    let audits = fixtures::audits(&db, org).await?;
    assert_eq!(audits.len(), 2);
    for (row, action, target_type, target, metadata) in [
        (
            &audits[0],
            "organization.created",
            "organization",
            json!(org.0),
            json!({}),
        ),
        (
            &audits[1],
            "organization.member.added",
            "actor",
            actor.clone(),
            json!({"role":"owner"}),
        ),
    ] {
        assert_eq!(row["action"], action);
        assert_eq!(row["actor_id"], *actor);
        assert_eq!(row["organization_id"], org.0.to_string());
        assert_eq!(row["target_type"], target_type);
        assert_eq!(row["target_id"], target);
        assert_eq!(row["metadata"], metadata);
    }
    assert_single_bootstrap(fixtures::counts(&db).await?);
    assert_no_secrets(&[EMAIL, PASSWORD]);
    db.finish().await
}

fn assert_single_bootstrap(counts: serde_json::Value) {
    for (table, count) in counts.as_object().unwrap() {
        let expected = match table.as_str() {
            "actors" | "users" | "organizations" | "organization_members" | "installation" => 1,
            "audit_events" => 2,
            _ => 0,
        };
        assert_eq!(count, expected, "{table}");
    }
}

fn second_answers(same: bool) -> Answers {
    let mut input = answers();
    if !same {
        input.email = "other@example.invalid".into();
        input.slug = "other".into();
    }
    input
}

#[tokio::test]
async fn repeated_bootstrap_exits_two_for_same_and_different_inputs_without_new_rows() -> Result {
    let db = Database::new().await?;
    execute(&db.app, answers()).await.unwrap();
    let before = fixtures::counts(&db).await?;
    for same in [true, false] {
        let error = execute(&db.app, second_answers(same)).await.unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("already bootstrapped"));
        assert_eq!(fixtures::counts(&db).await?, before);
        assert_no_secrets(&[EMAIL, PASSWORD, "other@example.invalid"]);
    }
    db.finish().await
}

#[tokio::test]
async fn concurrent_bootstrap_has_one_winner_for_same_and_different_inputs() -> Result {
    for same in [true, false] {
        let db = Database::new().await?;
        let mut lock = db.owner.begin().await?;
        database::installation::hold_memberships(&mut lock).await?;
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let mut tasks = Vec::new();
        for input in [answers(), second_answers(same)] {
            let pool = db.app.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                execute(&pool, input).await
            }));
        }
        barrier.wait().await;
        // One run holds the singleton and waits for membership; the other must
        // overlap it and wait for that uncommitted singleton before either can finish.
        database::wait_for_blocked_operations(&db.inspector, 2).await?;
        lock.rollback().await?;
        let mut winners = 0;
        for task in tasks {
            match task.await? {
                Ok(_) => winners += 1,
                Err(error) => {
                    assert_eq!(error.code, 2);
                    assert!(error.message.contains("already bootstrapped"));
                }
            }
        }
        assert_eq!(winners, 1);
        assert_single_bootstrap(fixtures::counts(&db).await?);
        assert_no_secrets(&[EMAIL, PASSWORD, "other@example.invalid"]);
        db.finish().await?;
    }
    Ok(())
}

#[tokio::test]
async fn membership_failure_rolls_back_the_organization_and_every_other_row() -> Result {
    let db = Database::new().await?;
    fixtures::refuse_membership(&db).await?;
    let before = fixtures::counts(&db).await?;
    let error = execute(&db.app, answers()).await.unwrap_err();
    assert_failure(&error, "failed", &[EMAIL, PASSWORD]);
    assert_eq!(fixtures::counts(&db).await?, before);
    db.finish().await
}

#[tokio::test]
async fn hash_failure_writes_nothing_and_discloses_no_input() -> Result {
    let db = Database::new().await?;
    let before = fixtures::counts(&db).await?;
    let error =
        bootstrap::execute_with_hasher(&db.app, answers(), |_| Err(auth::PasswordHashError))
            .await
            .unwrap_err();
    assert_failure(&error, "failed", &[EMAIL, PASSWORD]);
    assert_eq!(fixtures::counts(&db).await?, before);
    db.finish().await
}

#[tokio::test]
async fn bootstrapped_owner_signs_in_and_creates_an_invite_over_http() -> Result {
    let db = Database::new().await?;
    let org = execute(&db.app, answers()).await.unwrap();
    let server = Server::new(sessions::StateData::new(db.app.clone(), ORIGIN.into())).await?;
    let sign_in = server.sign_in().await?;
    assert_eq!(sign_in.status, 204);
    let cookie = sign_in.cookie()?;
    let response = server
        .request(
            "POST",
            &format!("/api/v1/organizations/{}/invites", org.0),
            Some(ORIGIN),
            Some(&cookie),
            r#"{"expires_in":3600}"#,
        )
        .await?;
    assert_eq!(response.status, 201);
    let invite: serde_json::Value = serde_json::from_str(&response.body)?;
    assert!(invite["token"].as_str().is_some());
    assert_no_secrets(&[EMAIL, PASSWORD, &cookie, invite["token"].as_str().unwrap()]);
    drop(server);
    db.finish().await
}

#[tokio::test]
async fn duplicate_email_names_only_the_field_and_rolls_back_every_new_row() -> Result {
    let db = Database::new().await?;
    let mut tx = db.app.begin().await?;
    db::create_person(
        &mut tx,
        EMAIL,
        database::PASSWORD_HASH,
        "Fixture existing person",
    )
    .await?;
    tx.commit().await?;
    let before = fixtures::counts(&db).await?;
    let error = execute(&db.app, answers()).await.unwrap_err();
    assert_failure(&error, "email", &[EMAIL, PASSWORD]);
    assert_eq!(fixtures::counts(&db).await?, before);
    db.finish().await
}
