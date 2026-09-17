mod support;
use support::{Database, Result};

#[tokio::test]
async fn migrations_reverse_empty_and_populated_databases() -> Result {
    let db = Database::new().await?;
    let migrator = sqlx::migrate!("../../migrations");
    for populated in [false, true] {
        if populated {
            sqlx::raw_sql(include_str!("fixtures/populated.sql"))
                .execute(&db.app)
                .await?;
        }
        migrator.undo(&db.owner, 202609170001).await?;
        let channel_tables: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname = 'public' AND tablename IN ('channels', 'channel_members', 'messages')").fetch_one(&db.owner).await?;
        assert_eq!(channel_tables, 0);
        let channel_audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE organization_id = $1 AND target_type = 'channel'")
            .bind(uuid::Uuid::from_u128(0x01995100_0000_7000_8000_000000000002)).fetch_one(&db.owner).await?;
        assert_eq!(channel_audits, i64::from(populated));
        migrator.run(&db.owner).await?;
        migrator.undo(&db.owner, 0).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname = 'public' AND tablename <> '_sqlx_migrations'").fetch_one(&db.owner).await?;
        assert_eq!(count, 0);
        migrator.run(&db.owner).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
            .fetch_one(&db.owner)
            .await?;
        assert!(count > 0);
    }
    db.finish().await
}

#[tokio::test]
async fn database_identity_version_and_migration_count_are_real() -> Result {
    let db = Database::new().await?;
    let (name, version): (String, i32) = sqlx::query_as(
        "SELECT current_database()::text, current_setting('server_version_num')::int / 10000",
    )
    .fetch_one(&db.owner)
    .await?;
    assert!(name.starts_with("fukulow_test_"));
    assert_eq!(version, 17);
    let migrations: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&db.owner)
        .await?;
    assert!(migrations > 0);
    db.finish().await
}
