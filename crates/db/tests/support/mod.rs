#![allow(
    dead_code,
    reason = "Each test crate uses a different subset of fixtures"
)]

pub(crate) mod installation;
pub(crate) mod invites;
pub(crate) mod sessions;

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
    pub(crate) inspector: PgPool,
    pub(crate) app: PgPool,
    url: String,
}

impl Database {
    pub(crate) async fn new() -> Result<Self> {
        let migrator_url = std::env::var("MIGRATOR_DATABASE_URL")
            .map_err(|_| "MIGRATOR_DATABASE_URL is required")?;
        let inspector_url = std::env::var("INSPECTOR_DATABASE_URL")
            .map_err(|_| "INSPECTOR_DATABASE_URL is required")?;
        let inspector_options = PgConnectOptions::from_str(&inspector_url)
            .map_err(|_| "invalid INSPECTOR_DATABASE_URL")?
            .disable_statement_logging();
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
        let inspector = pool(inspector_options.database(&database)).await?;
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT rolsuper FROM pg_roles WHERE rolname = current_user"
            )
            .fetch_one(&inspector)
            .await?
        );
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
        Ok(Self {
            owner,
            inspector,
            app,
            url,
        })
    }

    pub(crate) async fn finish(self) -> Result {
        self.app.close().await;
        self.owner.close().await;
        self.inspector.close().await;
        Postgres::drop_database(&self.url).await?;
        Ok(())
    }

    pub(crate) async fn person(&self) -> Result<ActorId> {
        let mut conn = self.app.begin().await?;
        let actor = db::create_person(
            &mut conn,
            &format!("{}@example.invalid", Uuid::now_v7()),
            PASSWORD_HASH,
            "Fixture person",
        )
        .await?;
        conn.commit().await?;
        Ok(actor)
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
            .bind(organization.0).bind(actor.0).bind(role).execute(&self.inspector).await?;
        Ok(())
    }

    pub(crate) async fn actor(
        &self,
        actor: ActorId,
    ) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
        let mut tx = self.app.begin().await?;
        sqlx::query("SELECT set_config('fukulow.actor_id', $1, true)")
            .bind(actor.0.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    pub(crate) async fn audit(&self, organization: OrganizationId) -> Result<Vec<Audit>> {
        let rows = sqlx::query("SELECT actor_id, action, target_type, target_id, metadata FROM audit_events WHERE organization_id = $1 ORDER BY action, target_id")
            .bind(organization.0).fetch_all(&self.inspector).await?;
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

pub(crate) struct Conversation {
    pub owner: ActorId,
    pub organization: OrganizationId,
    pub team: domain::TeamId,
    pub channel: domain::ChannelId,
}

impl Conversation {
    pub(crate) async fn new(db: &Database) -> Result<Self> {
        let owner = db.person().await?;
        let organization = db.organization(owner).await?;
        let team = db::create_team(&db.app, owner, organization, "Fixture team").await?;
        let channel = db::create_channel(
            &db.app,
            owner,
            organization,
            domain::ChannelScope::Team(team),
            "general",
        )
        .await?;
        Ok(Self {
            owner,
            organization,
            team,
            channel,
        })
    }

    pub(crate) async fn counter(&self, db: &Database) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT next_message_seq FROM channels WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.organization.0)
        .bind(self.channel.0)
        .fetch_one(&db.inspector)
        .await?)
    }

    pub(crate) async fn post(&self, db: &Database) -> Result<db::Message> {
        match db::post_message(
            &db.app,
            self.organization,
            self.channel,
            self.owner,
            message_id(),
            "Fixture body",
        )
        .await?
        {
            db::PostMessage::Posted(message) => Ok(message),
            db::PostMessage::AlreadyPosted(_) => Err("new id was already posted".into()),
        }
    }
}

pub(crate) fn message_id() -> domain::MessageId {
    domain::MessageId::from_client(Uuid::now_v7()).expect("UUID generator must produce v7")
}

pub(crate) async fn wait_for_blocked_operations(owner: &PgPool, count: i64) -> Result {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND usename = 'fukulow_app' AND cardinality(pg_blocking_pids(pid)) > 0")
                .fetch_one(owner).await?;
            if waiting >= count { return Ok::<_, sqlx::Error>(()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await??;
    Ok(())
}
