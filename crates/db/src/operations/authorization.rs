use domain::{ActorId, Capability, OrganizationId, OrganizationRole};
use sqlx::PgConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Allowed,
    Forbidden,
    NotAMember,
}

// Lock actor and organization first: taking membership first can deadlock with a
// demotion holding the organization while the invite INSERT waits on its foreign key.
pub(crate) async fn authorize(
    conn: &mut PgConnection,
    actor: ActorId,
    organization: OrganizationId,
    capability: Capability,
) -> Result<Access, sqlx::Error> {
    let row = sqlx::query!("SELECT role FROM organization_members WHERE organization_id = $1 AND actor_id = $2 AND status = 'active' FOR SHARE", organization.0, actor.0)
        .fetch_optional(conn).await?;
    let access = match row {
        None => Access::NotAMember,
        Some(row) => {
            let role = OrganizationRole::parse(&row.role)
                .ok_or(sqlx::Error::Protocol("invalid organization role".into()))?;
            if role.holds(capability) {
                Access::Allowed
            } else {
                Access::Forbidden
            }
        }
    };
    Ok(access)
}
