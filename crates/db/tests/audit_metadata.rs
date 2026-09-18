#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Clippy's test options do not cover integration-test helper functions"
)]

mod support;

use domain::{ActorId, OrganizationId};
use serde_json::{Value, json};
use support::{Database, Result};
use uuid::Uuid;

// Distinctive synthetic personal data makes accidental disclosure detectable. gitleaks:allow
const DISPLAY_NAME: &str = "Audit privacy sentinel 58 Qzx";
// Reserved domain; never a real person's address. gitleaks:allow
const EMAIL: &str = "audit-privacy-58-qzx@example.invalid";

fn uuid_cases() -> Vec<(Value, bool)> {
    let mut cases = Vec::new();
    for key in ["team_id", "channel_id", "invite_id"] {
        cases.push((json!({key: Uuid::now_v7().to_string()}), true));
        for value in [
            "01995100-0000-7000-a000-000000000002",
            "00000000-0000-0000-0000-000000000000",
        ] {
            cases.push((json!({key: value}), true));
        }
        for value in [
            "01995100-0000-7000-A000-000000000002",
            "0199510000007000a000000000000002",
            "{01995100-0000-7000-a000-000000000002}",
            "01995100-0000-7000-a000-000000000002\n",
            "01995100-0000-7000-g000-000000000002",
            DISPLAY_NAME,
            EMAIL,
        ] {
            cases.push((json!({key: value}), false));
        }
    }
    cases
}

fn enumeration_cases() -> Vec<(Value, bool)> {
    let mut cases = Vec::new();
    for (key, names) in [
        ("role", &["owner", "admin", "member", "manager"][..]),
        ("from", &["owner", "admin", "member", "manager"]),
        ("to", &["owner", "admin", "member", "manager"]),
        ("reason", &["removed", "left_service"]),
        ("scope", &["organization", "team"]),
        ("kind", &["organization", "team"]),
    ] {
        for name in names {
            cases.push((json!({key: name}), true));
        }
        for value in [DISPLAY_NAME, EMAIL, "", "OWNER"] {
            cases.push((json!({key: value}), false));
        }
    }
    cases
}

fn max_uses_cases() -> Vec<(Value, bool)> {
    let mut cases = Vec::new();
    for (number, accepted) in [
        (0, false),
        (1, true),
        (100, true),
        (101, false),
        (-1, false),
    ] {
        cases.push((json!({"max_uses": number}), accepted));
    }
    for (number, accepted) in [
        (1.5, false),
        (1.0, true),
        (1e2, true),
        (1e-2, false),
        (1e100, false),
    ] {
        cases.push((json!({"max_uses": number}), accepted));
    }
    for value in ["1", DISPLAY_NAME, EMAIL] {
        cases.push((json!({"max_uses": value}), false));
    }
    cases
}

fn contract_cases() -> Vec<(Value, bool)> {
    let mut cases = vec![(json!({}), true)];
    cases.extend(uuid_cases());
    cases.extend(enumeration_cases());
    cases.extend(max_uses_cases());
    for key in [
        "team_id",
        "channel_id",
        "invite_id",
        "role",
        "from",
        "to",
        "reason",
        "scope",
        "kind",
        "max_uses",
    ] {
        for value in [
            Value::Null,
            json!(true),
            json!([]),
            json!({"name": DISPLAY_NAME}),
        ] {
            cases.push((json!({key: value}), false));
        }
        if key != "max_uses" {
            cases.push((json!({key: 1}), false));
        }
    }
    for value in [
        Value::Null,
        json!([]),
        json!(1),
        json!(true),
        json!(DISPLAY_NAME),
        json!({"unknown": "member"}),
        json!({"role": "member", "unknown": EMAIL}),
        json!({"role": "removed"}),
        json!({"reason": "owner"}),
        json!({"scope": "member"}),
        json!({"kind": "removed"}),
    ] {
        cases.push((value, false));
    }
    cases.push((json!({"team_id": Uuid::nil(), "channel_id": Uuid::nil(), "invite_id": Uuid::nil(), "role": "owner", "from": "member", "to": "manager", "reason": "removed", "scope": "team", "kind": "organization", "max_uses": 100}), true));
    cases
}

async fn insert_metadata(
    conn: &mut sqlx::PgConnection,
    organization: OrganizationId,
    actor: ActorId,
    metadata: &Value,
) -> std::result::Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query("INSERT INTO public.audit_events (id, organization_id, actor_id, action, target_type, target_id, metadata) VALUES ($1, $2, $3, 'team.created', 'team', $4, $5)")
        .bind(Uuid::now_v7()).bind(organization.0).bind(actor.0)
        .bind(Uuid::now_v7()).bind(metadata).execute(conn).await
}

fn assert_no_personal_data(value: &str) {
    for personal in [DISPLAY_NAME, EMAIL] {
        assert!(!value.contains(personal), "personal data was disclosed");
    }
}

fn assert_check_violation(error: &sqlx::Error) {
    let error = error.as_database_error().expect("database error");
    assert_eq!(error.code().as_deref(), Some("23514"));
    assert_eq!(error.constraint(), Some("audit_events_metadata_check"));
}

#[tokio::test]
async fn metadata_contract_is_enforced_for_every_writer() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    let org = db.organization(actor).await?;
    let before = db.audit(org).await?;
    for (case, (metadata, accepted)) in contract_cases().iter().enumerate() {
        // The inspector bypasses RLS: the CHECK must protect non-application writers too.
        for application in [true, false] {
            let mut tx = if application {
                db.actor(actor).await?
            } else {
                db.inspector.begin().await?
            };
            let result = insert_metadata(&mut tx, org, actor, metadata).await;
            if *accepted {
                assert!(result.is_ok(), "accepted metadata case {case}");
            } else {
                assert!(result.is_err(), "forbidden metadata case {case} inserted");
                let error = result.unwrap_err();
                assert_check_violation(&error);
                if !application {
                    let detail = error
                        .as_database_error()
                        .unwrap()
                        .downcast_ref::<sqlx::postgres::PgDatabaseError>()
                        .detail()
                        .expect("inspector sees the rejected row");
                    for personal in [DISPLAY_NAME, EMAIL] {
                        // PostgreSQL truncates long column values in error details.
                        if metadata.as_object().is_some_and(|object| {
                            object.len() == 1
                                && object.values().any(|v| v.as_str() == Some(personal))
                        }) {
                            assert!(detail.contains(personal));
                        }
                    }
                }
                let error = db::CreateTeamError::Database(error);
                assert_no_personal_data(&format!("{error} {error:?}"));
            }
            tx.rollback().await?;
        }
    }
    let after = db.audit(org).await?;
    assert_eq!(after, before);
    assert_no_personal_data(&format!("{after:?}"));
    db.finish().await
}

#[tokio::test]
async fn metadata_rejections_hide_personal_data_and_roll_back_the_operation() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    let org = db.organization(actor).await?;
    let before = db.audit(org).await?;
    // Simulate a writer copying free text into an allowed key, after the real
    // constructor runs, so this exercises the operation's error and rollback path.
    sqlx::raw_sql(
        "CREATE FUNCTION public.inject_audit_personal_data() RETURNS trigger
         LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
         BEGIN
             NEW.metadata := jsonb_build_object('role', (
                 SELECT name FROM public.teams
                 WHERE organization_id = NEW.organization_id AND id = NEW.target_id));
             RETURN NEW;
         END $$;
         CREATE TRIGGER inject_audit_personal_data BEFORE INSERT ON public.audit_events
             FOR EACH ROW EXECUTE FUNCTION public.inject_audit_personal_data();",
    )
    .execute(&db.owner)
    .await?;
    for personal in [DISPLAY_NAME, EMAIL] {
        let error = db::create_team(&db.app, actor, org, personal)
            .await
            .unwrap_err();
        let db::CreateTeamError::Database(ref database_error) = error else {
            panic!("expected the operation's Database error");
        };
        assert_check_violation(database_error);
        assert_no_personal_data(&format!("{error} {error:?}"));
        assert!(std::error::Error::source(&error).is_none());
    }
    let after = db.audit(org).await?;
    assert_eq!(after, before);
    assert_no_personal_data(&format!("{after:?}"));
    let teams: i64 = sqlx::query_scalar("SELECT count(*) FROM teams WHERE organization_id = $1")
        .bind(org.0)
        .fetch_one(&db.inspector)
        .await?;
    assert_eq!(teams, 0);
    db.finish().await
}

#[tokio::test]
async fn metadata_function_is_hardened_and_never_returns_null() -> Result {
    let db = Database::new().await?;
    let properties: (String, Vec<String>, bool) = sqlx::query_as(
        "SELECT provolatile::text, proconfig, prosecdef FROM pg_proc
         WHERE oid = 'public.fukulow_audit_metadata_valid(jsonb)'::regprocedure",
    )
    .fetch_one(&db.owner)
    .await?;
    assert_eq!(
        properties,
        (
            "i".into(),
            vec!["search_path=pg_catalog, pg_temp".into()],
            false
        )
    );
    let validated: bool = sqlx::query_scalar(
        "SELECT convalidated FROM pg_constraint WHERE conrelid = 'public.audit_events'::regclass
         AND conname = 'audit_events_metadata_check'",
    )
    .fetch_one(&db.owner)
    .await?;
    assert!(validated);
    for (metadata, expected) in contract_cases()
        .into_iter()
        .map(|(value, valid)| (Some(value), valid))
        .chain([(None, false)])
    {
        let result: Option<bool> =
            sqlx::query_scalar("SELECT public.fukulow_audit_metadata_valid($1)")
                .bind(metadata)
                .fetch_one(&db.app)
                .await?;
        assert_eq!(result, Some(expected));
    }
    db.finish().await
}
