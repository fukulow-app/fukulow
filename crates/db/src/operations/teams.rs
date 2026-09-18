use super::{
    AddTeamMemberError, ChangeTeamMemberRoleError, CreateTeamError, RemoveTeamMemberError,
    errors::constraint, lock_actor, lock_organization,
};
use crate::audit::{Event, RemovalReason};
use crate::context::{Context, begin};
use domain::{ActorId, MemberStatus, OrganizationId, TeamId, TeamRole};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn create_team(
    pool: &PgPool,
    performed_by: ActorId,
    organization_id: OrganizationId,
    name: &str,
) -> Result<TeamId, CreateTeamError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    lock_actor(&mut tx, performed_by, performed_by).await?;
    if !lock_organization(&mut tx, organization_id, super::OrganizationLock::Update).await? {
        return Err(CreateTeamError::NotFound);
    }
    let team = TeamId(Uuid::now_v7());
    sqlx::query!(
        "INSERT INTO teams (id, organization_id, name) VALUES ($1, $2, $3)",
        team.0,
        organization_id.0,
        name
    )
    .execute(&mut *tx)
    .await?;
    Event::team_created(team)
        .write(&mut tx, organization_id, performed_by)
        .await?;
    tx.commit().await?;
    Ok(team)
}

pub async fn add_team_member(
    pool: &PgPool,
    performed_by: ActorId,
    organization_id: OrganizationId,
    team_id: TeamId,
    actor_id: ActorId,
    role: TeamRole,
) -> Result<(), AddTeamMemberError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    let departed = lock_actor(&mut tx, actor_id, performed_by).await?;
    if !lock_organization(&mut tx, organization_id, super::OrganizationLock::Update).await? {
        return Err(AddTeamMemberError::NotFound);
    }
    let team = sqlx::query!(
        "SELECT id FROM teams WHERE organization_id = $1 AND id = $2",
        organization_id.0,
        team_id.0
    )
    .fetch_optional(&mut *tx)
    .await?;
    let member = sqlx::query!(
        "SELECT status FROM organization_members WHERE organization_id = $1 AND actor_id = $2",
        organization_id.0,
        actor_id.0
    )
    .fetch_optional(&mut *tx)
    .await?;
    if team.is_none() {
        return Err(AddTeamMemberError::NotFound);
    }
    let member = member.ok_or(AddTeamMemberError::NotFound)?;
    if departed == Some(true) {
        return Err(AddTeamMemberError::ActorDeparted);
    }
    let status = MemberStatus::parse(&member.status)
        .ok_or_else(|| sqlx::Error::Protocol("invalid stored membership status".into()))?;
    if status != MemberStatus::Active {
        return Err(AddTeamMemberError::NotAnActiveMember);
    }
    sqlx::query!("INSERT INTO team_members (team_id, actor_id, organization_id, role) VALUES ($1, $2, $3, $4)", team_id.0, actor_id.0, organization_id.0, role.as_str())
        .execute(&mut *tx).await.map_err(|error| {
            if constraint(&error, "team_members_pkey") { AddTeamMemberError::AlreadyMember } else { error.into() }
        })?;
    Event::team_member_added(actor_id, team_id, role)
        .write(&mut tx, organization_id, performed_by)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn change_team_member_role(
    pool: &PgPool,
    performed_by: ActorId,
    organization_id: OrganizationId,
    team_id: TeamId,
    actor_id: ActorId,
    role: TeamRole,
) -> Result<(), ChangeTeamMemberRoleError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    lock_actor(&mut tx, performed_by, performed_by).await?;
    if !lock_organization(&mut tx, organization_id, super::OrganizationLock::Update).await? {
        return Err(ChangeTeamMemberRoleError::NotFound);
    }
    let row = sqlx::query!("SELECT role FROM team_members WHERE organization_id = $1 AND team_id = $2 AND actor_id = $3", organization_id.0, team_id.0, actor_id.0)
        .fetch_optional(&mut *tx).await?.ok_or(ChangeTeamMemberRoleError::NotFound)?;
    let from = TeamRole::parse(&row.role)
        .ok_or_else(|| sqlx::Error::Protocol("invalid stored team role".into()))?;
    if from == role {
        tx.commit().await?;
        return Ok(());
    }
    sqlx::query!("UPDATE team_members SET role = $4 WHERE organization_id = $1 AND team_id = $2 AND actor_id = $3", organization_id.0, team_id.0, actor_id.0, role.as_str())
        .execute(&mut *tx).await?;
    Event::team_member_role_changed(actor_id, team_id, from, role)
        .write(&mut tx, organization_id, performed_by)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn remove_team_member(
    pool: &PgPool,
    performed_by: ActorId,
    organization_id: OrganizationId,
    team_id: TeamId,
    actor_id: ActorId,
) -> Result<(), RemoveTeamMemberError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    lock_actor(&mut tx, performed_by, performed_by).await?;
    if !lock_organization(&mut tx, organization_id, super::OrganizationLock::Update).await? {
        return Err(RemoveTeamMemberError::NotFound);
    }
    let result = sqlx::query!(
        "DELETE FROM team_members WHERE organization_id = $1 AND team_id = $2 AND actor_id = $3",
        organization_id.0,
        team_id.0,
        actor_id.0
    )
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(RemoveTeamMemberError::NotFound);
    }
    Event::team_member_removed(actor_id, team_id, RemovalReason::Removed)
        .write(&mut tx, organization_id, performed_by)
        .await?;
    tx.commit().await?;
    Ok(())
}
