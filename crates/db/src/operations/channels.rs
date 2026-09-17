use super::{
    AddChannelMemberError, CreateChannelError, errors::constraint, lock_actor, lock_organization,
};
use crate::audit::Event;
use crate::context::{Context, begin};
use domain::{ActorId, ChannelId, ChannelScope, OrganizationId};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn create_channel(
    pool: &PgPool,
    performed_by: ActorId,
    organization_id: OrganizationId,
    scope: ChannelScope,
    name: &str,
) -> Result<ChannelId, CreateChannelError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    lock_actor(&mut tx, performed_by, performed_by).await?;
    if !lock_organization(&mut tx, organization_id).await? {
        return Err(CreateChannelError::NotFound);
    }
    let team_id = match scope {
        ChannelScope::Organization => None,
        ChannelScope::Team(team) => {
            if sqlx::query!(
                "SELECT id FROM teams WHERE organization_id = $1 AND id = $2",
                organization_id.0,
                team.0
            )
            .fetch_optional(&mut *tx)
            .await?
            .is_none()
            {
                return Err(CreateChannelError::NotFound);
            }
            Some(team.0)
        }
    };
    let channel = ChannelId(Uuid::now_v7());
    sqlx::query!("INSERT INTO channels (id, organization_id, team_id, scope, name) VALUES ($1, $2, $3, $4, $5)", channel.0, organization_id.0, team_id, scope.as_str(), name)
        .execute(&mut *tx).await.map_err(|error| {
            if constraint(&error, "channels_team_name_key") || constraint(&error, "channels_organization_name_key") {
                CreateChannelError::NameTaken
            } else { error.into() }
        })?;
    Event::channel_created(channel, scope)
        .write(&mut tx, organization_id, performed_by)
        .await?;
    tx.commit().await?;
    Ok(channel)
}

pub async fn add_channel_member(
    pool: &PgPool,
    performed_by: ActorId,
    organization_id: OrganizationId,
    channel_id: ChannelId,
    actor_id: ActorId,
) -> Result<(), AddChannelMemberError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    let departed = lock_actor(&mut tx, actor_id, performed_by).await?;
    if !lock_organization(&mut tx, organization_id).await? {
        return Err(AddChannelMemberError::NotFound);
    }
    let row = sqlx::query!(
        "SELECT scope FROM channels WHERE organization_id = $1 AND id = $2",
        organization_id.0,
        channel_id.0
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AddChannelMemberError::NotFound)?;
    if row.scope == ChannelScope::Organization.as_str() {
        return Err(AddChannelMemberError::OrganizationScoped);
    }
    match departed {
        None => return Err(AddChannelMemberError::NotFound),
        Some(true) => return Err(AddChannelMemberError::ActorDeparted),
        Some(false) => {}
    }
    sqlx::query!("INSERT INTO channel_members (channel_id, channel_scope, organization_id, actor_id) SELECT id, scope, organization_id, $3 FROM channels WHERE organization_id = $1 AND id = $2", organization_id.0, channel_id.0, actor_id.0)
        .execute(&mut *tx).await.map_err(|error| {
            if constraint(&error, "channel_members_pkey") { AddChannelMemberError::AlreadyMember } else { error.into() }
        })?;
    Event::channel_member_added(actor_id, channel_id)
        .write(&mut tx, organization_id, performed_by)
        .await?;
    tx.commit().await?;
    Ok(())
}
