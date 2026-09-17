#![allow(dead_code)]

use domain::{ActorId, OrganizationId};
use sqlx::{
    ConnectOptions, PgPool, Postgres, Row,
    migrate::MigrateDatabase,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

pub(crate) type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
// Deliberately not a usable PHC hash: these tests exercise storage, not authentication. gitleaks:allow
pub(crate) const PASSWORD_HASH: &str = "fixture-password-hash";

pub(crate) struct Database {
    pub(crate) owner: PgPool,
    pub(crate) app: PgPool,
    url: String,
}

impl Database {
    pub(crate) async fn new() -> Result<Self> {
        let migrator_url = std::env::var("MIGRATOR_DATABASE_URL")
            .map_err(|_| "MIGRATOR_DATABASE_URL is required")?;
        let app_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
        let database = format!("fukulow_test_{}", Uuid::now_v7().simple());
        let migrator = PgConnectOptions::from_str(&migrator_url)
            .map_err(|_| "invalid MIGRATOR_DATABASE_URL")?
            .database(&database)
            .disable_statement_logging();
        let application = PgConnectOptions::from_str(&app_url)
            .map_err(|_| "invalid DATABASE_URL")?
            .database(&database)
            .disable_statement_logging();
        let url = migrator.to_url_lossy().to_string();
        Postgres::create_database(&url)
            .await
            .map_err(|_| "cannot create test database using MIGRATOR_DATABASE_URL")?;
        let owner = pool(migrator).await?;
        sqlx::migrate!("../../migrations").run(&owner).await?;
        let app = pool(application).await?;
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT current_user::text")
                .fetch_one(&app)
                .await?,
            "fukulow_app"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT current_user::text")
                .fetch_one(&owner)
                .await?,
            "fukulow_migrator"
        );
        Ok(Self { owner, app, url })
    }

    pub(crate) async fn finish(self) -> Result {
        self.app.close().await;
        self.owner.close().await;
        Postgres::drop_database(&self.url).await?;
        Ok(())
    }

    pub(crate) async fn person(&self) -> Result<ActorId> {
        let mut conn = self.app.acquire().await?;
        Ok(db::create_person(
            &mut conn,
            &format!("{}@example.invalid", Uuid::now_v7()),
            PASSWORD_HASH,
            "Fixture person",
        )
        .await?)
    }

    pub(crate) async fn organization(&self, actor: ActorId) -> Result<OrganizationId> {
        Ok(db::create_organization(
            &self.app,
            actor,
            "Fixture organization",
            &Uuid::now_v7().simple().to_string(),
        )
        .await?)
    }

    pub(crate) async fn member(
        &self,
        organization: OrganizationId,
        actor: ActorId,
        role: &str,
    ) -> Result {
        sqlx::query("INSERT INTO organization_members (organization_id, actor_id, role, display_name) VALUES ($1, $2, $3, 'Fixture override')")
            .bind(organization.0).bind(actor.0).bind(role).execute(&self.app).await?;
        Ok(())
    }

    pub(crate) async fn audit(&self, organization: OrganizationId) -> Result<Vec<Audit>> {
        let rows = sqlx::query("SELECT actor_id, action, target_type, target_id, metadata FROM audit_events WHERE organization_id = $1 ORDER BY action, target_id")
            .bind(organization.0).fetch_all(&self.owner).await?;
        rows.into_iter()
            .map(|row| {
                Ok(Audit {
                    actor: row.try_get("actor_id")?,
                    action: row.try_get("action")?,
                    target_type: row.try_get("target_type")?,
                    target: row.try_get("target_id")?,
                    metadata: row.try_get("metadata")?,
                })
            })
            .collect()
    }
}

async fn pool(options: PgConnectOptions) -> Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .after_connect(|conn, _| {
            Box::pin(async move {
                // A lock-order regression must fail instead of leaving CI hung indefinitely.
                sqlx::query("SET statement_timeout = '10s'")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .map_err(|_| "test database connection failed")?)
}

#[derive(Debug, PartialEq)]
pub(crate) struct Audit {
    pub(crate) actor: Uuid,
    pub(crate) action: String,
    pub(crate) target_type: String,
    pub(crate) target: Uuid,
    pub(crate) metadata: serde_json::Value,
}

pub(crate) fn rejected<T>(result: std::result::Result<T, sqlx::Error>, code: &str) {
    let error = match result {
        Ok(_) => panic!("database accepted a forbidden statement"),
        Err(error) => error,
    };
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some(code),
        "unexpected database error code"
    );
}
