use super::authorization::{Access, authorize};
use super::{
    AcceptInviteError, AccessError, CreateInviteError, CreatePersonError, RevokeInviteError,
};
use crate::{
    TokenHash,
    audit::Event,
    context::{Context, begin},
};
use domain::{ActorId, Capability, InviteId, InviteKind, OrganizationId};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// `new_token` is called only once the capability is granted, as `sign_in` calls its own
/// only once the credential is accepted: a caller without `InviteMember` is 404 or 403
/// even when the RNG would fail, and a refused request makes no token.
pub async fn create_invite<T: Send>(
    pool: &PgPool,
    performed_by: ActorId,
    organization: OrganizationId,
    new_token: impl FnOnce() -> Option<(T, TokenHash)> + Send,
    expires_at: OffsetDateTime,
    max_uses: i32,
) -> Result<(InviteId, T), CreateInviteError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    super::lock_actor(&mut tx, performed_by, performed_by).await?;
    if !super::lock_organization(&mut tx, organization, super::OrganizationLock::Share).await? {
        return Err(CreateInviteError::NotAMember);
    }
    match authorize(
        &mut tx,
        performed_by,
        organization,
        Capability::InviteMember,
    )
    .await?
    {
        Access::Allowed => {}
        Access::Forbidden => return Err(CreateInviteError::Forbidden),
        Access::NotAMember => return Err(CreateInviteError::NotAMember),
    }
    let (token, token_hash) = new_token().ok_or(CreateInviteError::TokenUnavailable)?;
    let id = InviteId(Uuid::now_v7());
    let inserted = sqlx::query!("INSERT INTO invites (id, organization_id, kind, token_hash, expires_at, max_uses, created_by_actor_id) SELECT $1, id, 'organization', $3, $4, $5, $6 FROM organizations WHERE id = $2 RETURNING id", id.0, organization.0, token_hash.as_str(), expires_at, max_uses, performed_by.0)
        .fetch_optional(&mut *tx).await?;
    if inserted.is_none() {
        return Err(CreateInviteError::NotAMember);
    }
    Event::invite_created(organization, id, max_uses)
        .write(&mut tx, organization, performed_by)
        .await?;
    tx.commit().await?;
    Ok((id, token))
}

/// `invite` is `None` when the path did not hold a UUID. It is still decided after the
/// capability, so a malformed invite id cannot hide a 404 or 403 for the caller.
pub async fn revoke_invite(
    pool: &PgPool,
    performed_by: ActorId,
    organization: OrganizationId,
    invite: Option<InviteId>,
) -> Result<(), RevokeInviteError> {
    let mut tx = begin(pool, Context::Actor(performed_by)).await?;
    super::lock_actor(&mut tx, performed_by, performed_by).await?;
    if !super::lock_organization(&mut tx, organization, super::OrganizationLock::Share).await? {
        return Err(RevokeInviteError::NotAMember);
    }
    match authorize(
        &mut tx,
        performed_by,
        organization,
        Capability::InviteMember,
    )
    .await?
    {
        Access::Allowed => {}
        Access::Forbidden => return Err(RevokeInviteError::Forbidden),
        Access::NotAMember => return Err(RevokeInviteError::NotAMember),
    }
    let invite = invite.ok_or(RevokeInviteError::NotFound)?;
    let row = sqlx::query!(
        "SELECT revoked_at FROM invites WHERE organization_id = $1 AND id = $2 FOR UPDATE",
        organization.0,
        invite.0
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RevokeInviteError::NotFound)?;
    if row.revoked_at.is_none() {
        sqlx::query!(
            "UPDATE invites SET revoked_at = now() WHERE organization_id = $1 AND id = $2",
            organization.0,
            invite.0
        )
        .execute(&mut *tx)
        .await?;
        Event::invite_revoked(organization, invite)
            .write(&mut tx, organization, performed_by)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn invite_is_usable(pool: &PgPool, token_hash: &TokenHash) -> Result<bool, AccessError> {
    let mut tx = begin(pool, Context::InviteToken(token_hash.clone())).await?;
    let usable = sqlx::query!("SELECT id FROM invites WHERE token_hash = $1 AND kind = 'organization' AND used_count < max_uses AND revoked_at IS NULL AND expires_at > now()", token_hash.as_str())
        .fetch_optional(&mut *tx).await?.is_some();
    tx.commit().await?;
    Ok(usable)
}

pub async fn accept_invite(
    pool: &PgPool,
    token_hash: &TokenHash,
    email: &str,
    password_hash: &str,
    display_name: &str,
) -> Result<(ActorId, OrganizationId), AcceptInviteError> {
    let mut tx = begin(pool, Context::InviteToken(token_hash.clone())).await?;
    let row = sqlx::query!("UPDATE invites SET used_count = used_count + 1 WHERE token_hash = $1 AND used_count < max_uses AND revoked_at IS NULL AND expires_at > now() RETURNING id, organization_id, kind", token_hash.as_str())
        .fetch_optional(&mut *tx).await?.ok_or(AcceptInviteError::InviteUnusable)?;
    if !accepts_new_person(&row.kind) {
        return Err(AcceptInviteError::InviteUnusable);
    }
    let organization = OrganizationId(row.organization_id);
    let actor = super::create_person(&mut tx, email, password_hash, display_name)
        .await
        .map_err(|error| match error {
            CreatePersonError::EmailTaken => AcceptInviteError::EmailTaken,
            CreatePersonError::Database(error) => AcceptInviteError::Database(error),
        })?;
    sqlx::query!("INSERT INTO organization_members (organization_id, actor_id, role, status) VALUES ($1, $2, 'member', 'active')", organization.0, actor.0).execute(&mut *tx).await?;
    Event::member_invited(actor, InviteId(row.id))
        .write(&mut tx, organization, actor)
        .await?;
    tx.commit().await?;
    Ok((actor, organization))
}

fn accepts_new_person(kind: &str) -> bool {
    match InviteKind::parse(kind) {
        Some(InviteKind::Organization) => true,
        Some(InviteKind::Team) | None => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_organization_invites_admit_new_people() {
        assert!(super::accepts_new_person("organization"));
        for kind in ["team", "channel", "connection", "unknown", ""] {
            assert!(!super::accepts_new_person(kind));
        }
    }
}
