use super::{CreateOrganizationError, CreatePersonError, errors::constraint, lock_actor};
use crate::audit::Event;
use crate::context::{Context, begin};
use domain::{ActorId, OrganizationId, OrganizationRole};
use sqlx::{Acquire, PgPool, Postgres, Transaction};
use uuid::Uuid;

pub async fn create_person(
    conn: &mut Transaction<'_, Postgres>,
    email: &str,
    password_hash: &str,
    display_name: &str,
) -> Result<ActorId, CreatePersonError> {
    // A savepoint also removes the actor if the email conflicts inside an invite transaction.
    let mut tx = conn.begin().await?;
    let actor = ActorId(Uuid::now_v7());
    sqlx::query!(
        "INSERT INTO actors (id, type, display_name) VALUES ($1, 'human', $2)",
        actor.0,
        display_name
    )
    .execute(&mut *tx)
    .await?;
    crate::context::set_context(&mut tx, Context::Actor(actor)).await?;
    sqlx::query!("INSERT INTO users (actor_id, actor_type, email, password_hash) VALUES ($1, 'human', $2, $3)", actor.0, email, password_hash)
        .execute(&mut *tx).await.map_err(|error| {
            if constraint(&error, "users_email_lower_key") { CreatePersonError::EmailTaken } else { error.into() }
        })?;
    tx.commit().await?;
    Ok(actor)
}

pub async fn create_organization(
    pool: &PgPool,
    creator: ActorId,
    name: &str,
    slug: &str,
) -> Result<OrganizationId, CreateOrganizationError> {
    let mut tx = begin(pool, Context::Actor(creator)).await?;
    match lock_actor(&mut tx, creator, creator).await? {
        None => return Err(CreateOrganizationError::ActorNotFound),
        Some(true) => return Err(CreateOrganizationError::ActorDeparted),
        Some(false) => {}
    }
    let organization = OrganizationId(Uuid::now_v7());
    sqlx::query!(
        "INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)",
        organization.0,
        name,
        slug
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        if constraint(&error, "organizations_slug_format") {
            CreateOrganizationError::InvalidSlug
        } else if constraint(&error, "organizations_slug_key") {
            CreateOrganizationError::SlugTaken
        } else {
            error.into()
        }
    })?;
    sqlx::query!("INSERT INTO organization_members (organization_id, actor_id, role) VALUES ($1, $2, 'owner')", organization.0, creator.0).execute(&mut *tx).await?;
    Event::organization_created(organization)
        .write(&mut tx, organization, creator)
        .await?;
    Event::member_added(creator, OrganizationRole::Owner)
        .write(&mut tx, organization, creator)
        .await?;
    tx.commit().await?;
    Ok(organization)
}
