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
use domain::ActorId;
use support::{Database, Result, installation::counts, rejected};
use uuid::Uuid;

async fn bootstrap(
    db: &Database,
    email: &str,
    slug: &str,
) -> std::result::Result<domain::OrganizationId, db::BootstrapError> {
    db::bootstrap_installation(
        &db.app,
        ActorId(Uuid::now_v7()),
        "Fixture organization",
        slug,
        "Fixture owner",
        email,
        support::PASSWORD_HASH,
    )
    .await
}

#[tokio::test]
async fn database_field_errors_roll_back_every_table() -> Result {
    let db = Database::new().await?;
    let before = counts(&db).await?;
    assert!(matches!(
        bootstrap(&db, "owner@example.invalid", "INVALID").await,
        Err(db::BootstrapError::InvalidSlug)
    ));
    assert_eq!(counts(&db).await?, before);
    let person = db.person().await?;
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE actor_id = $1")
        .bind(person.0)
        .fetch_one(&db.inspector)
        .await?;
    let before = counts(&db).await?;
    assert!(matches!(
        bootstrap(&db, &email, "fixture").await,
        Err(db::BootstrapError::EmailTaken)
    ));
    assert_eq!(counts(&db).await?, before);
    db.finish().await
}

#[tokio::test]
async fn installation_constraints_and_policy_refuse_other_actors_and_multiple_rows() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    let other = db.person().await?;
    rejected(
        sqlx::query("INSERT INTO installation (bootstrapped_by_actor_id) VALUES ($1)")
            .bind(actor.0)
            .execute(&db.app)
            .await,
        "42501",
    );
    let mut tx = db.actor(other).await?;
    rejected(
        sqlx::query("INSERT INTO installation (bootstrapped_by_actor_id) VALUES ($1)")
            .bind(actor.0)
            .execute(&mut *tx)
            .await,
        "42501",
    );
    tx.rollback().await?;
    let mut tx = db.actor(actor).await?;
    rejected(
        sqlx::query(
            "INSERT INTO installation (singleton, bootstrapped_by_actor_id) VALUES (false, $1)",
        )
        .bind(actor.0)
        .execute(&mut *tx)
        .await,
        "23514",
    );
    tx.rollback().await?;
    let mut tx = db.actor(actor).await?;
    sqlx::query("INSERT INTO installation (bootstrapped_by_actor_id) VALUES ($1)")
        .bind(actor.0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut tx = db.actor(other).await?;
    rejected(
        sqlx::query("INSERT INTO installation (bootstrapped_by_actor_id) VALUES ($1)")
            .bind(other.0)
            .execute(&mut *tx)
            .await,
        "23505",
    );
    tx.rollback().await?;
    db.finish().await
}

#[tokio::test]
async fn unrelated_organization_cannot_read_bootstrapped_rows() -> Result {
    let db = Database::new().await?;
    let organization = bootstrap(&db, "owner@example.invalid", "fixture").await?;
    let other = db.person().await?;
    db.organization(other).await?;
    let mut tx = db.actor(other).await?;
    for query in [
        "SELECT count(*) FROM organizations WHERE id = $1",
        "SELECT count(*) FROM organization_members WHERE organization_id = $1",
        "SELECT count(*) FROM actors a JOIN organization_members m ON m.actor_id = a.id WHERE m.organization_id = $1",
    ] {
        let count: i64 = sqlx::query_scalar(query)
            .bind(organization.0)
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(count, 0);
    }
    rejected(
        sqlx::query("SELECT * FROM audit_events WHERE organization_id = $1")
            .bind(organization.0)
            .fetch_all(&mut *tx)
            .await,
        "42501",
    );
    tx.rollback().await?;
    db.finish().await
}

#[tokio::test]
async fn counts_cover_every_application_table() -> Result {
    let db = Database::new().await?;
    let tables: Vec<String> = sqlx::query_scalar("SELECT tablename::text FROM pg_tables WHERE schemaname = 'public' AND tablename <> '_sqlx_migrations' ORDER BY tablename")
        .fetch_all(&db.inspector).await?;
    assert_eq!(
        counts(&db)
            .await?
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        tables
    );
    db.finish().await
}
