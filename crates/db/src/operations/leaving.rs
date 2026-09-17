use super::{LeaveServiceError, active_owners};
use crate::audit::{Event, RemovalReason};
use crate::context::{Context, begin};
use domain::{ActorId, ChannelId, OrganizationId, OwnerChange, TeamId, check_owner_change};
use sqlx::{PgConnection, PgPool};

pub async fn leave_service(pool: &PgPool, actor_id: ActorId) -> Result<(), LeaveServiceError> {
    let mut tx = begin(pool, Context::Actor(actor_id)).await?;
    let actor = sqlx::query!(
        "SELECT deleted_at FROM actors WHERE id = $1 FOR UPDATE",
        actor_id.0
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(LeaveServiceError::NotFound)?;
    if actor.deleted_at.is_some() {
        return Err(LeaveServiceError::NotFound);
    }
    // The actor lock closes reactivation; only active organizations need the owner lock.
    let organizations = sqlx::query!("SELECT id FROM organizations o WHERE EXISTS (SELECT 1 FROM organization_members m WHERE m.organization_id = o.id AND m.actor_id = $1 AND m.status = 'active') ORDER BY id FOR UPDATE", actor_id.0)
        .fetch_all(&mut *tx).await?;
    for row in organizations {
        let organization = OrganizationId(row.id);
        let owners = active_owners(&mut tx, organization).await?;
        check_owner_change(&owners, OwnerChange::Leave(actor_id)).map_err(|_| {
            LeaveServiceError::LastOwner {
                organization_id: organization,
            }
        })?;
    }
    // Departure spans organizations: enumerate only this actor's own listings, including inactive ones.
    let memberships = sqlx::query!("SELECT organization_id, status FROM organization_members WHERE actor_id = $1 ORDER BY organization_id", actor_id.0)
        .fetch_all(&mut *tx).await?;
    let channel_organizations = sqlx::query!("SELECT DISTINCT organization_id FROM channel_members WHERE actor_id = $1 ORDER BY organization_id", actor_id.0)
        .fetch_all(&mut *tx).await?;
    for row in &memberships {
        let organization = OrganizationId(row.organization_id);
        leave_organization(&mut tx, organization, actor_id).await?;
        if row.status != "left" {
            Event::member_left(actor_id)
                .write(&mut tx, organization, actor_id)
                .await?;
        }
    }
    for row in channel_organizations {
        leave_channels(&mut tx, OrganizationId(row.organization_id), actor_id).await?;
    }
    // The next statement after a status transition loses ordinary tenant visibility.
    for row in memberships {
        sqlx::query!("UPDATE organization_members SET status = 'left', display_name = NULL WHERE organization_id = $1 AND actor_id = $2", row.organization_id, actor_id.0)
            .execute(&mut *tx).await?;
    }
    sqlx::query!("DELETE FROM users WHERE actor_id = $1", actor_id.0)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "UPDATE actors SET display_name = NULL, deleted_at = now() WHERE id = $1",
        actor_id.0
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn leave_organization(
    conn: &mut PgConnection,
    organization: OrganizationId,
    actor: ActorId,
) -> Result<(), sqlx::Error> {
    let removed = sqlx::query!(
        "DELETE FROM team_members WHERE organization_id = $1 AND actor_id = $2 RETURNING team_id",
        organization.0,
        actor.0
    )
    .fetch_all(&mut *conn)
    .await?;
    for row in removed {
        Event::team_member_removed(actor, TeamId(row.team_id), RemovalReason::LeftService)
            .write(conn, organization, actor)
            .await?;
    }
    Ok(())
}

async fn leave_channels(
    conn: &mut PgConnection,
    organization: OrganizationId,
    actor: ActorId,
) -> Result<(), sqlx::Error> {
    // Audit before deleting: an outside actor's departure audit is admitted only while its
    // own listing still names the channel and organization.
    let listings = sqlx::query!("SELECT channel_id FROM channel_members WHERE organization_id = $1 AND actor_id = $2 ORDER BY channel_id", organization.0, actor.0).fetch_all(&mut *conn).await?;
    for row in &listings {
        Event::channel_member_left(actor, ChannelId(row.channel_id))
            .write(conn, organization, actor)
            .await?;
    }
    sqlx::query!(
        "DELETE FROM channel_members WHERE organization_id = $1 AND actor_id = $2",
        organization.0,
        actor.0
    )
    .execute(conn)
    .await?;
    Ok(())
}
