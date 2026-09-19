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
use sqlx::Row;
use std::collections::BTreeSet;
use support::{Database, Result};
use uuid::Uuid;

const TABLES: &[&str] = &[
    "actors",
    "audit_events",
    "channel_members",
    "channels",
    "installation",
    "invites",
    "messages",
    "organization_members",
    "organizations",
    "sessions",
    "team_members",
    "teams",
    "users",
];

#[tokio::test]
async fn nullable_columns_are_exactly_the_documented_exceptions() -> Result {
    let db = Database::new().await?;
    let columns: Vec<(String, String)> = sqlx::query_as("SELECT table_name, column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name <> '_sqlx_migrations' AND is_nullable = 'YES' ORDER BY table_name, column_name")
        .fetch_all(&db.owner).await?;
    assert_eq!(
        columns,
        [
            ("actors", "deleted_at"),
            ("actors", "display_name"),
            ("channels", "team_id"),
            ("invites", "revoked_at"),
            ("invites", "target_team_id"),
            ("organization_members", "display_name"),
            ("sessions", "revoked_at"),
        ]
        .map(|(t, c)| (t.to_owned(), c.to_owned()))
    );
    db.finish().await
}

#[tokio::test]
async fn defaults_and_timestamp_types_are_exact() -> Result {
    let db = Database::new().await?;
    let rows = sqlx::query("SELECT table_name, column_name, column_default, data_type FROM information_schema.columns WHERE table_schema = 'public' AND table_name <> '_sqlx_migrations'").fetch_all(&db.owner).await?;
    let mut timestamps = BTreeSet::new();
    for row in rows {
        let table: String = row.try_get("table_name")?;
        let column: String = row.try_get("column_name")?;
        let default: Option<String> = row.try_get("column_default")?;
        let expected = match (table.as_str(), column.as_str()) {
            ("installation", "singleton") => Some("true"),
            (_, "created_at") => Some("now()"),
            ("organization_members" | "team_members", "role") => Some("'member'::text"),
            ("organization_members", "status") => Some("'active'::text"),
            ("invites", "used_count") | ("channels", "next_message_seq") => Some("0"),
            _ => None,
        };
        assert_eq!(default.as_deref(), expected, "{table}.{column}");
        let data_type: String = row.try_get("data_type")?;
        assert_ne!(data_type, "timestamp without time zone");
        if column.ends_with("_at") {
            assert_eq!(data_type, "timestamp with time zone", "{table}.{column}");
            if column == "created_at" {
                timestamps.insert(table);
            }
        }
    }
    assert_eq!(timestamps, TABLES.iter().map(|s| s.to_string()).collect());
    db.finish().await
}

#[tokio::test]
async fn composite_foreign_keys_cannot_be_disabled_by_nulls() -> Result {
    let db = Database::new().await?;
    let rows: Vec<(String, String, bool)> = sqlx::query_as("SELECT c.relname::text, a.attname::text, a.attnotnull FROM pg_constraint k JOIN pg_class c ON c.oid = k.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace CROSS JOIN LATERAL unnest(k.conkey) key(attnum) JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = key.attnum WHERE n.nspname = 'public' AND k.contype = 'f' AND cardinality(k.conkey) > 1 ORDER BY 1, 2")
        .fetch_all(&db.owner).await?;
    assert_eq!(
        rows.iter()
            .map(|(table, column, _)| (table.as_str(), column.as_str()))
            .collect::<Vec<_>>(),
        [
            ("channel_members", "channel_id"),
            ("channel_members", "channel_scope"),
            ("channel_members", "organization_id"),
            ("channels", "organization_id"),
            ("channels", "team_id"),
            ("invites", "organization_id"),
            ("invites", "target_team_id"),
            ("messages", "channel_id"),
            ("messages", "organization_id"),
            ("team_members", "actor_id"),
            ("team_members", "organization_id"),
            ("team_members", "organization_id"),
            ("team_members", "team_id"),
            ("users", "actor_id"),
            ("users", "actor_type"),
        ]
    );
    for (table, column, required) in rows {
        assert_eq!(
            required,
            !((table == "invites" && column == "target_team_id")
                || (table == "channels" && column == "team_id")),
            "{table}.{column}"
        );
    }
    db.finish().await
}

#[tokio::test]
async fn every_actor_and_organization_reference_has_a_foreign_key() -> Result {
    let db = Database::new().await?;
    let missing: Vec<(String, String)> = sqlx::query_as("SELECT c.relname::text, a.attname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped WHERE n.nspname = 'public' AND c.relkind = 'r' AND (a.attname = 'organization_id' OR a.attname = 'actor_id' OR a.attname LIKE '%\\_actor_id' ESCAPE '\') AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conrelid = c.oid AND k.contype = 'f' AND a.attnum = ANY(k.conkey))")
        .fetch_all(&db.owner).await?;
    assert!(missing.is_empty(), "{missing:?}");
    db.finish().await
}

#[tokio::test]
async fn actors_are_global_and_names_have_only_the_published_locations() -> Result {
    let db = Database::new().await?;
    let names: Vec<(String, String)> = sqlx::query_as("SELECT table_name, column_name FROM information_schema.columns WHERE table_schema = 'public' AND column_name ILIKE '%name%' ORDER BY table_name, column_name")
        .fetch_all(&db.owner).await?;
    assert_eq!(
        names,
        [
            ("actors", "display_name"),
            ("channels", "name"),
            ("organization_members", "display_name"),
            ("organizations", "name"),
            ("teams", "name")
        ]
        .map(|(t, c)| (t.to_owned(), c.to_owned()))
    );
    let scoped: i64 = sqlx::query_scalar("SELECT count(*) FROM information_schema.columns WHERE table_schema = 'public' AND table_name IN ('actors', 'users', 'sessions', 'installation') AND column_name = 'organization_id'").fetch_one(&db.owner).await?;
    assert_eq!(scoped, 0);
    db.finish().await
}

fn updatable(table: &str, column: &str) -> bool {
    matches!(
        (table, column),
        ("actors", "display_name" | "deleted_at")
            | ("channels", "next_message_seq")
            | ("organizations", "name")
            | ("organization_members", "role" | "status" | "display_name")
            | ("team_members", "role")
            | ("invites", "used_count" | "revoked_at")
            | ("sessions", "revoked_at")
    )
}

#[tokio::test]
async fn a_foreign_key_to_deletable_rows_leads_an_index() -> Result {
    let db = Database::new().await?;
    let unindexed = unindexed_foreign_keys_to_deletable_rows(&db).await?;
    assert!(unindexed.is_empty(), "{unindexed:?}");
    db.finish().await
}

#[tokio::test]
async fn partial_and_invalid_indexes_do_not_serve_a_foreign_key() -> Result {
    let db = Database::new().await?;
    let reported = vec![("sessions".to_owned(), "sessions_actor_id_fkey".to_owned())];
    sqlx::query("DROP INDEX sessions_actor_id_idx")
        .execute(&db.owner)
        .await?;
    assert_eq!(
        unindexed_foreign_keys_to_deletable_rows(&db).await?,
        reported
    );
    sqlx::query(
        "CREATE INDEX sessions_actor_id_partial ON sessions (actor_id) WHERE revoked_at IS NULL",
    )
    .execute(&db.owner)
    .await?;
    assert_eq!(
        unindexed_foreign_keys_to_deletable_rows(&db).await?,
        reported
    );
    // A concurrent build that fails leaves its index behind, marked invalid.
    let actor = db.person().await?;
    for _ in 0..2 {
        sqlx::query("INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, $2, $3, now())")
            .bind(Uuid::now_v7())
            .bind(actor.0)
            .bind(Uuid::now_v7().to_string())
            .execute(&db.inspector)
            .await?;
    }
    assert!(
        sqlx::query(
            "CREATE UNIQUE INDEX CONCURRENTLY sessions_actor_id_invalid ON sessions (actor_id)"
        )
        .execute(&db.owner)
        .await
        .is_err()
    );
    let invalid: bool = sqlx::query_scalar(
        "SELECT NOT indisvalid FROM pg_index WHERE indexrelid = 'sessions_actor_id_invalid'::regclass",
    )
    .fetch_one(&db.owner)
    .await?;
    assert!(invalid);
    assert_eq!(
        unindexed_foreign_keys_to_deletable_rows(&db).await?,
        reported
    );
    db.finish().await
}

async fn unindexed_foreign_keys_to_deletable_rows(db: &Database) -> Result<Vec<(String, String)>> {
    // PostgreSQL does not index the referencing columns of a foreign key. Without one, every
    // delete from the referenced table scans the referencing table to check or cascade. Rows
    // the application can delete are those it holds DELETE on, and those a cascade from them
    // reaches. An invalid index, left by a failed concurrent build, is never used.
    Ok(sqlx::query_as(
        "WITH RECURSIVE deletable(relid) AS (
            SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = 'public' AND c.relkind = 'r'
                AND has_table_privilege('fukulow_app', c.oid, 'DELETE')
            UNION
            SELECT k.conrelid FROM pg_constraint k JOIN deletable d ON d.relid = k.confrelid
            WHERE k.contype = 'f' AND k.confdeltype = 'c'
        )
        SELECT k.conrelid::regclass::text, k.conname::text
        FROM pg_constraint k JOIN deletable d ON d.relid = k.confrelid
        WHERE k.contype = 'f' AND NOT EXISTS (
            SELECT 1 FROM pg_index i
            WHERE i.indrelid = k.conrelid AND i.indisvalid
                AND i.indpred IS NULL AND i.indexprs IS NULL
                AND i.indnkeyatts >= cardinality(k.conkey)
                AND (SELECT array_agg(x ORDER BY x) FROM unnest((i.indkey::int2[])[0:cardinality(k.conkey) - 1]) x)
                    = (SELECT array_agg(x ORDER BY x) FROM unnest(k.conkey) x)
        )
        ORDER BY 1, 2",
    )
    .fetch_all(&db.owner)
    .await?)
}

#[tokio::test]
async fn application_privileges_are_exact_for_every_table_and_column() -> Result {
    let db = Database::new().await?;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename::text FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename",
    )
    .fetch_all(&db.owner)
    .await?;
    let mut expected = vec!["_sqlx_migrations"];
    expected.extend_from_slice(TABLES);
    assert_eq!(tables, expected);
    for table in tables {
        for privilege in [
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "TRUNCATE",
            "REFERENCES",
            "TRIGGER",
            "MAINTAIN",
        ] {
            let granted: bool =
                sqlx::query_scalar("SELECT has_table_privilege('fukulow_app', $1, $2)")
                    .bind(&table)
                    .bind(privilege)
                    .fetch_one(&db.owner)
                    .await?;
            let expected = match privilege {
                "SELECT" => !matches!(
                    table.as_ref(),
                    "audit_events" | "installation" | "_sqlx_migrations"
                ),
                "INSERT" => table != "_sqlx_migrations",
                "DELETE" => {
                    table == "users" || table == "team_members" || table == "channel_members"
                }
                _ => false,
            };
            assert_eq!(granted, expected, "{table}: {privilege}");
        }
        check_columns(&db, &table).await?;
    }
    let roles: Vec<(String, bool, bool)> = sqlx::query_as("SELECT rolname::text, rolsuper, rolbypassrls FROM pg_roles WHERE rolname IN ('fukulow_app', 'fukulow_migrator') ORDER BY rolname").fetch_all(&db.owner).await?;
    assert_eq!(
        roles,
        vec![
            ("fukulow_app".into(), false, false),
            ("fukulow_migrator".into(), false, false)
        ]
    );
    let owners: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT tableowner::text FROM pg_tables WHERE schemaname = 'public'",
    )
    .fetch_all(&db.owner)
    .await?;
    assert_eq!(owners, ["fukulow_migrator"]);
    db.finish().await
}

async fn check_columns(db: &Database, table: &str) -> Result {
    let columns: Vec<String> = sqlx::query_scalar("SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1").bind(table).fetch_all(&db.owner).await?;
    for column in columns {
        for privilege in ["SELECT", "INSERT", "UPDATE", "REFERENCES"] {
            let granted: bool =
                sqlx::query_scalar("SELECT has_column_privilege('fukulow_app', $1, $2, $3)")
                    .bind(table)
                    .bind(&column)
                    .bind(privilege)
                    .fetch_one(&db.owner)
                    .await?;
            let expected = match privilege {
                "SELECT" => !matches!(table, "audit_events" | "installation" | "_sqlx_migrations"),
                "INSERT" => table != "_sqlx_migrations",
                "UPDATE" => updatable(table, &column),
                _ => false,
            };
            assert_eq!(granted, expected, "{table}.{column}: {privilege}");
        }
    }
    Ok(())
}
