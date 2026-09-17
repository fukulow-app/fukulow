use super::{LeaveServiceError, active_owners};
use crate::audit::{Event, RemovalReason};
use domain::{ActorId, OrganizationId, OwnerChange, TeamId, check_owner_change};
use sqlx::{PgConnection, PgPool};

pub async fn leave_service(pool: &PgPool, actor_id: ActorId) -> Result<(), LeaveServiceError> {
    let mut tx = pool.begin().await?;
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
    // The actor lock closes membership additions; the common order prevents two leavers deadlocking.
    let organizations = sqlx::query!("SELECT id FROM organizations o WHERE EXISTS (SELECT 1 FROM organization_members m WHERE m.organization_id = o.id AND m.actor_id = $1) ORDER BY id FOR UPDATE", actor_id.0)
        .fetch_all(&mut *tx).await?;
    for row in organizations {
        let organization = OrganizationId(row.id);
        let owners = active_owners(&mut tx, organization).await?;
        check_owner_change(&owners, OwnerChange::Leave(actor_id)).map_err(|_| {
            LeaveServiceError::LastOwner {
                organization_id: organization,
            }
        })?;
        leave_organization(&mut tx, organization, actor_id).await?;
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
    let changed = sqlx::query!("UPDATE organization_members SET status = 'left', display_name = NULL WHERE organization_id = $1 AND actor_id = $2 AND status <> 'left'", organization.0, actor.0)
        .execute(&mut *conn).await?;
    if changed.rows_affected() != 0 {
        Event::member_left(actor)
            .write(conn, organization, actor)
            .await?;
    }
    // A previously left membership can still carry an override until service departure.
    sqlx::query!("UPDATE organization_members SET display_name = NULL WHERE organization_id = $1 AND actor_id = $2", organization.0, actor.0).execute(conn).await?;
    Ok(())
}
