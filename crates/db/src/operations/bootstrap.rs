use super::{BootstrapError, errors::constraint};
use crate::audit::Event;
use crate::context::{Context, begin};
use domain::{ActorId, OrganizationId, OrganizationRole};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn bootstrap_installation(
    pool: &PgPool,
    actor: ActorId,
    organization_name: &str,
    slug: &str,
    display_name: &str,
    email: &str,
    password_hash: &str,
) -> Result<OrganizationId, BootstrapError> {
    let mut tx = begin(pool, Context::Actor(actor)).await?;
    sqlx::query!(
        "INSERT INTO actors (id, type, display_name) VALUES ($1, 'human', $2)",
        actor.0,
        display_name
    )
    .execute(&mut *tx)
    .await?;
    // The singleton must precede email and slug uniqueness checks so every repeated
    // run has the same outcome, including one waiting for an uncommitted bootstrap.
    sqlx::query!(
        "INSERT INTO installation (bootstrapped_by_actor_id) VALUES ($1)",
        actor.0
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        if constraint(&error, "installation_pkey") {
            BootstrapError::AlreadyBootstrapped
        } else {
            error.into()
        }
    })?;
    sqlx::query!("INSERT INTO users (actor_id, actor_type, email, password_hash) VALUES ($1, 'human', $2, $3)", actor.0, email, password_hash)
        .execute(&mut *tx).await.map_err(|error| {
            if constraint(&error, "users_email_lower_key") { BootstrapError::EmailTaken } else { error.into() }
        })?;
    let organization = OrganizationId(Uuid::now_v7());
    sqlx::query!(
        "INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)",
        organization.0,
        organization_name,
        slug
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        if constraint(&error, "organizations_slug_format") {
            BootstrapError::InvalidSlug
        } else {
            error.into()
        }
    })?;
    sqlx::query!("INSERT INTO organization_members (organization_id, actor_id, role) VALUES ($1, $2, 'owner')", organization.0, actor.0)
        .execute(&mut *tx).await?;
    Event::organization_created(organization)
        .write(&mut tx, organization, actor)
        .await?;
    Event::member_added(actor, OrganizationRole::Owner)
        .write(&mut tx, organization, actor)
        .await?;
    tx.commit().await?;
    Ok(organization)
}
