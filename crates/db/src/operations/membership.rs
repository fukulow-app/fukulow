use super::{
    ChangeMemberRoleError, ReactivateMemberError, SuspendMemberError, active_owners, lock_actor,
    lock_organization,
};
use crate::audit::Event;
use crate::context::{Context, begin};
use domain::{
    ActorId, MemberStatus, OrganizationId, OrganizationRole, OwnerChange, check_owner_change,
};
use sqlx::{PgConnection, PgPool};

async fn member(
    conn: &mut PgConnection,
    organization: OrganizationId,
    actor: ActorId,
) -> Result<Option<(OrganizationRole, MemberStatus)>, sqlx::Error> {
    let row = sqlx::query!("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2", organization.0, actor.0)
        .fetch_optional(conn).await?;
    row.map(|row| {
        let role = OrganizationRole::parse(&row.role)
            .ok_or_else(|| sqlx::Error::Protocol("invalid stored organization role".into()))?;
        let status = MemberStatus::parse(&row.status)
            .ok_or_else(|| sqlx::Error::Protocol("invalid stored membership status".into()))?;
        Ok((role, status))
    })
    .transpose()
}

pub async fn change_member_role(
    pool: &PgPool,
    performed_by: ActorId,
    organization_id: OrganizationId,
    actor_id: ActorId,
    role: OrganizationRole,
) -> Result<(), ChangeMemberRoleError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    lock_actor(&mut tx, performed_by, performed_by).await?;
    if !lock_organization(&mut tx, organization_id).await? {
        return Err(ChangeMemberRoleError::NotFound);
    }
    let (from, status) = member(&mut tx, organization_id, actor_id)
        .await?
        .ok_or(ChangeMemberRoleError::NotFound)?;
    // Someone who has left is no longer a member; the row stays only as history. Changing
    // their role would write an audit record, which cannot be removed, about a membership
    // that grants nothing.
    if status == MemberStatus::Left {
        return Err(ChangeMemberRoleError::NotFound);
    }
    if from == role {
        tx.commit().await?;
        return Ok(());
    }
    if status == MemberStatus::Active
        && from == OrganizationRole::Owner
        && role != OrganizationRole::Owner
    {
        check_owner_change(
            &active_owners(&mut tx, organization_id).await?,
            OwnerChange::Demote(actor_id),
        )?;
    }
    sqlx::query!(
        "UPDATE organization_members SET role = $3 WHERE organization_id = $1 AND actor_id = $2",
        organization_id.0,
        actor_id.0,
        role.as_str()
    )
    .execute(&mut *tx)
    .await?;
    Event::member_role_changed(actor_id, from, role)
        .write(&mut tx, organization_id, performed_by)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn suspend_member(
    pool: &PgPool,
    performed_by: ActorId,
    organization_id: OrganizationId,
    actor_id: ActorId,
) -> Result<(), SuspendMemberError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    lock_actor(&mut tx, performed_by, performed_by).await?;
    if !lock_organization(&mut tx, organization_id).await? {
        return Err(SuspendMemberError::NotFound);
    }
    let (role, status) = member(&mut tx, organization_id, actor_id)
        .await?
        .ok_or(SuspendMemberError::NotFound)?;
    if status != MemberStatus::Active {
        return Err(SuspendMemberError::NotActive);
    }
    if role == OrganizationRole::Owner {
        check_owner_change(
            &active_owners(&mut tx, organization_id).await?,
            OwnerChange::Suspend(actor_id),
        )?;
    }
    sqlx::query!("UPDATE organization_members SET status = 'suspended' WHERE organization_id = $1 AND actor_id = $2", organization_id.0, actor_id.0)
        .execute(&mut *tx).await?;
    Event::member_suspended(actor_id)
        .write(&mut tx, organization_id, performed_by)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn reactivate_member(
    pool: &PgPool,
    performed_by: ActorId,
    organization_id: OrganizationId,
    actor_id: ActorId,
) -> Result<(), ReactivateMemberError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    let departed = lock_actor(&mut tx, actor_id, performed_by).await?;
    if !lock_organization(&mut tx, organization_id).await? {
        return Err(ReactivateMemberError::NotFound);
    }
    let (_, status) = member(&mut tx, organization_id, actor_id)
        .await?
        .ok_or(ReactivateMemberError::NotFound)?;
    if departed == Some(true) {
        return Err(ReactivateMemberError::ActorDeparted);
    }
    if status != MemberStatus::Suspended {
        return Err(ReactivateMemberError::NotSuspended);
    }
    sqlx::query!("UPDATE organization_members SET status = 'active' WHERE organization_id = $1 AND actor_id = $2", organization_id.0, actor_id.0)
        .execute(&mut *tx).await?;
    Event::member_reactivated(actor_id)
        .write(&mut tx, organization_id, performed_by)
        .await?;
    tx.commit().await?;
    Ok(())
}
