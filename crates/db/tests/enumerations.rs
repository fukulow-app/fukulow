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

use domain::{
    ActorType, ChannelScope, InviteKind, MemberStatus, OrganizationRole, TeamId, TeamRole,
};
use std::collections::BTreeSet;
use support::{Database, Result};
use uuid::Uuid;

#[tokio::test]
async fn canonical_names_equal_database_checks() -> Result {
    let db = Database::new().await?;
    let enumerations = [
        (
            "invites",
            "kind",
            InviteKind::ALL.iter().map(|v| v.as_str()).collect(),
        ),
        (
            "actors",
            "type",
            ActorType::ALL.iter().map(|v| v.as_str()).collect(),
        ),
        (
            "organization_members",
            "role",
            OrganizationRole::ALL.iter().map(|v| v.as_str()).collect(),
        ),
        (
            "organization_members",
            "status",
            MemberStatus::ALL.iter().map(|v| v.as_str()).collect(),
        ),
        (
            "team_members",
            "role",
            TeamRole::ALL.iter().map(|v| v.as_str()).collect(),
        ),
        (
            "channels",
            "scope",
            BTreeSet::from([
                ChannelScope::Organization.as_str(),
                ChannelScope::Team(TeamId(Uuid::nil())).as_str(),
            ]),
        ),
    ];
    let mut checks = Vec::new();
    for (table, column, names) in enumerations {
        let definitions: Vec<String> = sqlx::query_scalar(
            "SELECT pg_get_constraintdef(k.oid) FROM pg_constraint k
             JOIN pg_class c ON c.oid = k.conrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(k.conkey)
             WHERE n.nspname = 'public' AND k.contype = 'c'
               AND c.relname = $1 AND a.attname = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_all(&db.owner)
        .await?;
        checks.push((table, column, names, definitions));
    }
    db.finish().await?;
    for (table, column, names, definitions) in checks {
        assert_eq!(definitions.len(), 1, "{table}.{column}");
        let definition = &definitions[0];
        let stored: BTreeSet<_> = if column == "scope" {
            // Scope is part of a table CHECK; unrelated literals must not count as scopes.
            definition
                .split("scope = '")
                .skip(1)
                .map(|comparison| comparison.split_once('\'').unwrap().0)
                .collect()
        } else {
            definition.split('\'').skip(1).step_by(2).collect()
        };
        assert_eq!(names, stored, "{table}.{column}");
    }
    Ok(())
}

#[tokio::test]
async fn audit_metadata_names_equal_rust_enumerations() -> Result {
    let db = Database::new().await?;
    let definition: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('public.fukulow_audit_metadata_valid(jsonb)'::regprocedure)",
    )
    .fetch_one(&db.owner)
    .await?;
    let roles: BTreeSet<_> = OrganizationRole::ALL
        .iter()
        .map(|v| v.as_str())
        .chain(TeamRole::ALL.iter().map(|v| v.as_str()))
        .collect();
    // RemovalReason is crate-private and has no as_str; read its exhaustive match
    // arms rather than duplicate spellings that could drift from the constructors.
    let reasons: BTreeSet<_> = include_str!("../src/audit.rs")
        .split("RemovalReason::")
        .skip(1)
        .map(|arm| {
            let (_, value) = arm.split_once("=>").expect("RemovalReason match arm");
            value
                .trim()
                .strip_prefix('"')
                .expect("literal reason")
                .split_once('"')
                .expect("closed reason literal")
                .0
        })
        .collect();
    assert!(!reasons.is_empty());
    let scopes = BTreeSet::from([
        ChannelScope::Organization.as_str(),
        ChannelScope::Team(TeamId(Uuid::nil())).as_str(),
    ]);
    let kinds = InviteKind::ALL.iter().map(|v| v.as_str()).collect();
    for (key, expected) in [
        ("role", &roles),
        ("from", &roles),
        ("to", &roles),
        ("reason", &reasons),
        ("scope", &scopes),
        ("kind", &kinds),
    ] {
        // Include SQL-only literals as well as Rust names: probing just Rust would
        // miss a name added only to the database allow list.
        let candidates: BTreeSet<_> = definition
            .split('\'')
            .skip(1)
            .step_by(2)
            .chain(expected.iter().copied())
            .collect();
        let mut accepted = BTreeSet::new();
        for name in candidates {
            let valid: bool = sqlx::query_scalar(
                "SELECT public.fukulow_audit_metadata_valid(jsonb_build_object($1::text, $2::text))",
            ).bind(key).bind(name).fetch_one(&db.app).await?;
            if valid {
                accepted.insert(name);
            }
        }
        assert_eq!(&accepted, expected, "audit metadata key {key}");
    }
    db.finish().await
}
