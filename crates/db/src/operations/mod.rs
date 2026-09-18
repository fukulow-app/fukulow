mod channels;
mod messages;
pub use channels::{add_channel_member, create_channel};
pub use messages::{Message, PostMessage, messages_after, messages_before, post_message};
mod errors;
mod identity;
mod leaving;
mod membership;
mod teams;

pub use errors::*;
pub use identity::{create_organization, create_person};
pub use leaving::leave_service;
pub use membership::{change_member_role, reactivate_member, suspend_member};
pub use teams::{add_team_member, change_team_member_role, create_team, remove_team_member};

use domain::{ActorId, OrganizationId};
use sqlx::PgConnection;

async fn lock_organization(
    conn: &mut PgConnection,
    organization: OrganizationId,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query!(
        "SELECT id FROM organizations WHERE id = $1 FOR UPDATE",
        organization.0
    )
    .fetch_optional(conn)
    .await?
    .is_some())
}

async fn active_owners(
    conn: &mut PgConnection,
    organization: OrganizationId,
) -> Result<Vec<ActorId>, sqlx::Error> {
    Ok(sqlx::query!("SELECT actor_id FROM organization_members WHERE organization_id = $1 AND role = 'owner' AND status = 'active'", organization.0)
        .fetch_all(conn).await?.into_iter().map(|row| ActorId(row.actor_id)).collect())
}

// Audit foreign keys lock performed_by too. Taking both actors before the organization
// prevents an audit insert from deadlocking with the performer's departure.
async fn lock_actor(
    conn: &mut PgConnection,
    actor: ActorId,
    performed_by: ActorId,
) -> Result<Option<bool>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id, deleted_at FROM actors WHERE id = $1 OR id = $2 ORDER BY id FOR SHARE",
        actor.0,
        performed_by.0
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .find(|row| row.id == actor.0)
        .map(|row| row.deleted_at.is_some()))
}

mod sessions;
pub use sessions::{DUMMY_PASSWORD_HASH, Me, me, resolve_session, revoke_session, sign_in};
