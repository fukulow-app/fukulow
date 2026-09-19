use super::{Database, Result};
use domain::OrganizationId;
use serde_json::Value;

pub(crate) async fn counts(db: &Database) -> Result<Value> {
    Ok(sqlx::query_scalar(
        "SELECT jsonb_build_object(
        'actors', (SELECT count(*) FROM actors),
        'users', (SELECT count(*) FROM users),
        'organizations', (SELECT count(*) FROM organizations),
        'organization_members', (SELECT count(*) FROM organization_members),
        'teams', (SELECT count(*) FROM teams),
        'team_members', (SELECT count(*) FROM team_members),
        'channels', (SELECT count(*) FROM channels),
        'channel_members', (SELECT count(*) FROM channel_members),
        'messages', (SELECT count(*) FROM messages),
        'invites', (SELECT count(*) FROM invites),
        'sessions', (SELECT count(*) FROM sessions),
        'audit_events', (SELECT count(*) FROM audit_events),
        'installation', (SELECT count(*) FROM installation))",
    )
    .fetch_one(&db.inspector)
    .await?)
}

pub(crate) async fn stored(db: &Database, org: OrganizationId) -> Result<Value> {
    Ok(sqlx::query_scalar(
        "SELECT jsonb_build_object(
        'organization', to_jsonb(o), 'actor', to_jsonb(a), 'user', to_jsonb(u),
        'membership', to_jsonb(m), 'installation', to_jsonb(i))
        FROM organizations o
        JOIN organization_members m ON m.organization_id = o.id
        JOIN actors a ON a.id = m.actor_id
        JOIN users u ON u.actor_id = a.id
        JOIN installation i ON i.bootstrapped_by_actor_id = a.id
        WHERE o.id = $1 AND m.organization_id = $1",
    )
    .bind(org.0)
    .fetch_one(&db.inspector)
    .await?)
}

pub(crate) async fn audits(db: &Database, org: OrganizationId) -> Result<Vec<Value>> {
    Ok(sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM audit_events e WHERE organization_id = $1 ORDER BY id",
    )
    .bind(org.0)
    .fetch_all(&db.inspector)
    .await?)
}

pub(crate) async fn refuse_membership(db: &Database) -> Result {
    sqlx::query(
        "ALTER TABLE organization_members ADD CONSTRAINT fixture_reject_member CHECK (false)",
    )
    .execute(&db.owner)
    .await?;
    Ok(())
}

pub(crate) async fn hold_memberships(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result {
    sqlx::query("LOCK TABLE organization_members IN ACCESS EXCLUSIVE MODE")
        .execute(&mut **tx)
        .await?;
    Ok(())
}
