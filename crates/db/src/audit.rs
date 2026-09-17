use domain::{
    ActorId, ChannelId, ChannelScope, OrganizationId, OrganizationRole, TeamId, TeamRole,
};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub(crate) enum RemovalReason {
    Removed,
    LeftService,
}

pub(crate) struct Event {
    action: &'static str,
    target_type: &'static str,
    target_id: Uuid,
    metadata: Value,
}

impl Event {
    pub(crate) fn organization_created(id: OrganizationId) -> Self {
        Self {
            action: "organization.created",
            target_type: "organization",
            target_id: id.0,
            metadata: json!({}),
        }
    }
    pub(crate) fn member_added(id: ActorId, role: OrganizationRole) -> Self {
        Self {
            action: "organization.member.added",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({"role": role.as_str()}),
        }
    }
    pub(crate) fn member_role_changed(
        id: ActorId,
        from: OrganizationRole,
        to: OrganizationRole,
    ) -> Self {
        Self {
            action: "organization.member.role_changed",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({"from": from.as_str(), "to": to.as_str()}),
        }
    }
    pub(crate) fn member_suspended(id: ActorId) -> Self {
        Self {
            action: "organization.member.suspended",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({}),
        }
    }
    pub(crate) fn member_reactivated(id: ActorId) -> Self {
        Self {
            action: "organization.member.reactivated",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({}),
        }
    }
    pub(crate) fn member_left(id: ActorId) -> Self {
        Self {
            action: "organization.member.left",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({}),
        }
    }
    pub(crate) fn team_created(id: TeamId) -> Self {
        Self {
            action: "team.created",
            target_type: "team",
            target_id: id.0,
            metadata: json!({}),
        }
    }
    pub(crate) fn team_member_added(id: ActorId, team: TeamId, role: TeamRole) -> Self {
        Self {
            action: "team.member.added",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({"team_id": team.0, "role": role.as_str()}),
        }
    }
    pub(crate) fn team_member_role_changed(
        id: ActorId,
        team: TeamId,
        from: TeamRole,
        to: TeamRole,
    ) -> Self {
        Self {
            action: "team.member.role_changed",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({"team_id": team.0, "from": from.as_str(), "to": to.as_str()}),
        }
    }
    pub(crate) fn team_member_removed(id: ActorId, team: TeamId, reason: RemovalReason) -> Self {
        let reason = match reason {
            RemovalReason::Removed => "removed",
            RemovalReason::LeftService => "left_service",
        };
        Self {
            action: "team.member.removed",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({"team_id": team.0, "reason": reason}),
        }
    }
    pub(crate) fn channel_created(id: ChannelId, scope: ChannelScope) -> Self {
        let metadata = match scope {
            ChannelScope::Organization => json!({"scope": scope.as_str()}),
            ChannelScope::Team(team) => json!({"scope": scope.as_str(), "team_id": team.0}),
        };
        Self {
            action: "channel.created",
            target_type: "channel",
            target_id: id.0,
            metadata,
        }
    }
    pub(crate) fn channel_member_added(id: ActorId, channel: ChannelId) -> Self {
        Self {
            action: "channel.member.added",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({"channel_id": channel.0}),
        }
    }
    pub(crate) fn channel_member_left(id: ActorId, channel: ChannelId) -> Self {
        Self {
            action: "channel.member.removed",
            target_type: "actor",
            target_id: id.0,
            metadata: json!({"channel_id": channel.0, "reason": "left_service"}),
        }
    }
    pub(crate) async fn write(
        self,
        conn: &mut PgConnection,
        organization: OrganizationId,
        performed_by: ActorId,
    ) -> Result<(), sqlx::Error> {
        let id = Uuid::now_v7();
        // No RETURNING: the application deliberately cannot read audit history.
        sqlx::query!("INSERT INTO audit_events (id, organization_id, actor_id, action, target_type, target_id, metadata) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            id, organization.0, performed_by.0, self.action, self.target_type, self.target_id, self.metadata)
            .execute(conn).await?;
        Ok(())
    }
}
