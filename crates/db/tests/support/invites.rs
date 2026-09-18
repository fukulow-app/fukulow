use super::{Database, Result};
use domain::{ActorId, InviteId, OrganizationId};
use serde_json::Value;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

pub(crate) async fn invite(
    db: &Database,
    actor: ActorId,
    org: OrganizationId,
    hash: &db::TokenHash,
    max_uses: i32,
) -> Result<InviteId> {
    Ok(db::create_invite(
        &db.app,
        actor,
        org,
        hash,
        OffsetDateTime::now_utc() + Duration::hours(1),
        max_uses,
    )
    .await?)
}

pub(crate) async fn stored(db: &Database, org: OrganizationId, id: InviteId) -> Result<Value> {
    Ok(sqlx::query_scalar(
        "SELECT to_jsonb(i) FROM invites i WHERE organization_id = $1 AND id = $2",
    )
    .bind(org.0)
    .bind(id.0)
    .fetch_one(&db.inspector)
    .await?)
}

pub(crate) async fn counts(db: &Database) -> Result<(i64, i64, i64, i64, i64)> {
    Ok(sqlx::query_as("SELECT (SELECT count(*) FROM actors), (SELECT count(*) FROM users), (SELECT count(*) FROM organization_members), (SELECT count(*) FROM invites), (SELECT count(*) FROM audit_events)").fetch_one(&db.inspector).await?)
}

pub(crate) async fn joined(db: &Database, org: OrganizationId, email: &str) -> Result<Value> {
    Ok(sqlx::query_scalar("SELECT jsonb_build_object('actor', to_jsonb(a), 'user', to_jsonb(u), 'membership', to_jsonb(m)) FROM organization_members m JOIN actors a ON a.id = m.actor_id JOIN users u ON u.actor_id = a.id WHERE m.organization_id = $1 AND u.email = $2").bind(org.0).bind(email).fetch_one(&db.inspector).await?)
}

pub(crate) async fn set_role(
    db: &Database,
    org: OrganizationId,
    actor: ActorId,
    role: &str,
    status: &str,
) -> Result {
    sqlx::query("UPDATE organization_members SET role = $3, status = $4 WHERE organization_id = $1 AND actor_id = $2")
        .bind(org.0).bind(actor.0).bind(role).bind(status).execute(&db.inspector).await?;
    Ok(())
}

pub(crate) async fn consume(pool: &PgPool, org: OrganizationId, id: InviteId) -> Result {
    sqlx::query("UPDATE invites SET used_count = used_count + 1 WHERE organization_id = $1 AND id = $2 AND used_count < max_uses AND revoked_at IS NULL AND expires_at > now()")
        .bind(org.0).bind(id.0).execute(pool).await?;
    Ok(())
}

pub(crate) async fn unusable(
    db: &Database,
    actor: ActorId,
    org: OrganizationId,
    hash: &db::TokenHash,
    kind: &str,
) -> Result<Option<InviteId>> {
    match kind {
        "unknown" => Ok(None),
        "expired" => Ok(Some(
            db::create_invite(
                &db.app,
                actor,
                org,
                hash,
                OffsetDateTime::now_utc() - Duration::hours(1),
                1,
            )
            .await?,
        )),
        "revoked" | "exhausted" => {
            let id = invite(db, actor, org, hash, 1).await?;
            if kind == "revoked" {
                db::revoke_invite(&db.app, actor, org, id).await?;
            } else {
                consume(&db.inspector, org, id).await?;
            }
            Ok(Some(id))
        }
        "team" => {
            let team = db::create_team(&db.app, actor, org, "Fixture team").await?;
            let id = InviteId(Uuid::now_v7());
            sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, created_by_actor_id) VALUES ($1, $2, 'team', $3, $4, now() + interval '1 hour', 1, $5)")
                .bind(id.0).bind(org.0).bind(team.0).bind(hash.as_str()).bind(actor.0).execute(&db.inspector).await?;
            Ok(Some(id))
        }
        _ => Err("unknown fixture kind".into()),
    }
}

pub(crate) async fn fail_audit(db: &Database) -> Result {
    sqlx::raw_sql("CREATE FUNCTION fixture_fail_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture audit failure'; END $$; CREATE TRIGGER fixture_fail_audit BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION fixture_fail_audit();").execute(&db.inspector).await?;
    Ok(())
}

pub(crate) async fn assert_no_app_transaction(db: &Database) -> Result {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND usename = 'fukulow_app' AND xact_start IS NOT NULL")
        .fetch_one(&db.inspector).await?;
    assert_eq!(count, 0);
    Ok(())
}
