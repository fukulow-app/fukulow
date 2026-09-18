#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Clippy's test options do not cover integration-test helper functions"
)]

mod support;
use domain::{ActorId, OrganizationId, OrganizationRole as Role, TeamRole};
use serde_json::json;
use support::{Database, Result};
use uuid::Uuid;

#[tokio::test]
async fn leaving_erases_personal_data_memberships_and_sessions_but_preserves_history() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let actor = db.person().await?;
    let mut organizations = Vec::new();
    for _ in 0..2 {
        let org = db.organization(owner).await?;
        db.member(org, actor, "member").await?;
        let team = db::create_team(&db.app, owner, org, "Fixture").await?;
        db::add_team_member(&db.app, owner, org, team, actor, TeamRole::Member).await?;
        organizations.push((org, team));
    }
    sqlx::query(
        "INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, $2, $3, now())",
    )
    .bind(Uuid::now_v7())
    .bind(actor.0)
    .bind(Uuid::now_v7().to_string())
    .execute(&db.inspector)
    .await?;
    db::leave_service(&db.app, actor).await?;
    let retained: (Option<String>, bool) =
        sqlx::query_as("SELECT display_name, deleted_at IS NOT NULL FROM actors WHERE id = $1")
            .bind(actor.0)
            .fetch_one(&db.inspector)
            .await?;
    assert_eq!(retained, (None, true));
    let personal: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM users WHERE actor_id = $1), (SELECT count(*) FROM sessions WHERE actor_id = $1)").bind(actor.0).fetch_one(&db.inspector).await?;
    assert_eq!(personal, (0, 0));
    for (org, team) in organizations {
        assert_departed_membership(&db, org, actor).await?;
        let audits = db.audit(org).await?;
        let left: Vec<_> = audits.iter().filter(|a| a.actor == actor.0).collect();
        assert_eq!(left.len(), 2);
        assert_eq!(
            (
                &left[0].action,
                &left[0].target_type,
                left[0].target,
                &left[0].metadata
            ),
            (
                &"organization.member.left".into(),
                &"actor".into(),
                actor.0,
                &json!({})
            )
        );
        assert_eq!(
            (
                &left[1].action,
                &left[1].target_type,
                left[1].target,
                &left[1].metadata
            ),
            (
                &"team.member.removed".into(),
                &"actor".into(),
                actor.0,
                &json!({"team_id":team.0,"reason":"left_service"})
            )
        );
        let resolves: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events e JOIN actors a ON a.id = e.target_id WHERE e.organization_id = $1 AND e.target_type = 'actor' AND a.id = $2").bind(org.0).bind(actor.0).fetch_one(&db.inspector).await?;
        assert!(resolves > 0);
    }
    assert!(matches!(
        db::leave_service(&db.app, actor).await,
        Err(db::LeaveServiceError::NotFound)
    ));
    db.finish().await
}

async fn assert_departed_membership(db: &Database, org: OrganizationId, actor: ActorId) -> Result {
    let row: (String, Option<String>) = sqlx::query_as("SELECT status, display_name FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(org.0).bind(actor.0).fetch_one(&db.inspector).await?;
    assert_eq!(row, ("left".into(), None));
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM organization_members WHERE organization_id = $1 AND actor_id = $2 AND status = 'active')").bind(org.0).bind(actor.0).fetch_one(&db.inspector).await?;
    assert!(!active);
    let teams: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM team_members WHERE organization_id = $1 AND actor_id = $2",
    )
    .bind(org.0)
    .bind(actor.0)
    .fetch_one(&db.inspector)
    .await?;
    assert_eq!(teams, 0);
    Ok(())
}

#[tokio::test]
async fn leaving_without_organizations_writes_no_audit() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    db::leave_service(&db.app, actor).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events")
        .fetch_one(&db.inspector)
        .await?;
    assert_eq!(count, 0);
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE actor_id = $1)")
        .bind(actor.0)
        .fetch_one(&db.inspector)
        .await?;
    assert!(!exists);
    db.finish().await
}

#[tokio::test]
async fn already_left_membership_is_cleared_without_duplicate_audit() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let actor = db.person().await?;
    let org = db.organization(owner).await?;
    db.member(org, actor, "member").await?;
    sqlx::query("UPDATE organization_members SET status = 'left' WHERE organization_id = $1 AND actor_id = $2").bind(org.0).bind(actor.0).execute(&db.inspector).await?;
    let before = db.audit(org).await?;
    db::leave_service(&db.app, actor).await?;
    assert_departed_membership(&db, org, actor).await?;
    assert_eq!(db.audit(org).await?, before);
    db.finish().await
}

#[tokio::test]
async fn last_owner_in_any_organization_rolls_back_the_whole_departure() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let actor = db.person().await?;
    let first = db.organization(owner).await?;
    db.member(first, actor, "member").await?;
    let last = db.organization(actor).await?;
    let before = db.audit(first).await?;
    assert!(
        matches!(db::leave_service(&db.app, actor).await, Err(db::LeaveServiceError::LastOwner {organization_id}) if organization_id == last)
    );
    let active: bool = sqlx::query_scalar("SELECT status = 'active' FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(first.0).bind(actor.0).fetch_one(&db.inspector).await?;
    assert!(active);
    assert_eq!(db.audit(first).await?, before);
    db.finish().await
}

#[tokio::test]
async fn departed_or_inactive_actors_cannot_gain_memberships() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let actor = db.person().await?;
    let org = db.organization(owner).await?;
    db.member(org, actor, "member").await?;
    let team = db::create_team(&db.app, owner, org, "Fixture").await?;
    db::suspend_member(&db.app, owner, org, actor).await?;
    assert!(matches!(
        db::add_team_member(&db.app, owner, org, team, actor, TeamRole::Member).await,
        Err(db::AddTeamMemberError::NotAnActiveMember)
    ));
    sqlx::query("UPDATE organization_members SET status = 'left' WHERE organization_id = $1 AND actor_id = $2").bind(org.0).bind(actor.0).execute(&db.inspector).await?;
    assert!(matches!(
        db::reactivate_member(&db.app, owner, org, actor).await,
        Err(db::ReactivateMemberError::NotSuspended)
    ));
    db::leave_service(&db.app, actor).await?;
    assert!(matches!(
        db::create_organization(&db.app, actor, "Fixture", "departed").await,
        Err(db::CreateOrganizationError::ActorDeparted)
    ));
    assert!(matches!(
        db::add_team_member(&db.app, owner, org, team, actor, TeamRole::Member).await,
        Err(db::AddTeamMemberError::ActorDeparted)
    ));
    assert!(matches!(
        db::reactivate_member(&db.app, owner, org, actor).await,
        Err(db::ReactivateMemberError::ActorDeparted)
    ));
    db.finish().await
}

#[tokio::test]
async fn a_left_membership_cannot_change_role() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let left = db.person().await?;
    let departed = db.person().await?;
    let org = db.organization(owner).await?;
    db.member(org, left, "member").await?;
    db.member(org, departed, "member").await?;
    sqlx::query("UPDATE organization_members SET status = 'left' WHERE organization_id = $1 AND actor_id = $2").bind(org.0).bind(left.0).execute(&db.inspector).await?;
    db::leave_service(&db.app, departed).await?;
    let before = db.audit(org).await?;
    for actor in [left, departed] {
        for role in [Role::Owner, Role::Admin, Role::Member] {
            assert!(matches!(
                db::change_member_role(&db.app, owner, org, actor, role).await,
                Err(db::ChangeMemberRoleError::NotFound)
            ));
        }
        let membership: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(org.0).bind(actor.0).fetch_one(&db.inspector).await?;
        assert_eq!(membership, ("member".into(), "left".into()));
    }
    assert_eq!(db.audit(org).await?, before);
    db.finish().await
}

#[tokio::test]
async fn departure_clears_active_suspended_and_left_memberships_with_exact_audits() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    let mut conversations = Vec::new();
    for status in ["active", "suspended", "left"] {
        let c = support::Conversation::new(&db).await?;
        db.member(c.organization, actor, "admin").await?;
        db::add_team_member(
            &db.app,
            c.owner,
            c.organization,
            c.team,
            actor,
            domain::TeamRole::Member,
        )
        .await?;
        db::add_channel_member(&db.app, c.owner, c.organization, c.channel, actor).await?;
        sqlx::query("UPDATE organization_members SET status = $3 WHERE organization_id = $1 AND actor_id = $2").bind(c.organization.0).bind(actor.0).bind(status).execute(&db.inspector).await?;
        conversations.push((c, status));
    }
    db::leave_service(&db.app, actor).await?;
    for (c, old_status) in conversations {
        let (status, role, name): (String, String, Option<String>) = sqlx::query_as("SELECT status, role, display_name FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(c.organization.0).bind(actor.0).fetch_one(&db.inspector).await?;
        assert_eq!(
            (status.as_str(), role.as_str(), name),
            ("left", "admin", None)
        );
        let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM team_members WHERE organization_id = $1 AND actor_id = $2), (SELECT count(*) FROM channel_members WHERE organization_id = $1 AND actor_id = $2)").bind(c.organization.0).bind(actor.0).fetch_one(&db.inspector).await?;
        assert_eq!(counts, (0, 0));
        let events: Vec<_> = db
            .audit(c.organization)
            .await?
            .into_iter()
            .filter(|e| e.actor == actor.0)
            .collect();
        let mut expected = vec![support::Audit {
            actor: actor.0,
            action: "channel.member.removed".into(),
            target_type: "actor".into(),
            target: actor.0,
            metadata: serde_json::json!({"channel_id": c.channel.0, "reason": "left_service"}),
        }];
        if old_status != "left" {
            expected.push(support::Audit {
                actor: actor.0,
                action: "organization.member.left".into(),
                target_type: "actor".into(),
                target: actor.0,
                metadata: serde_json::json!({}),
            });
        }
        expected.push(support::Audit {
            actor: actor.0,
            action: "team.member.removed".into(),
            target_type: "actor".into(),
            target: actor.0,
            metadata: serde_json::json!({"team_id": c.team.0, "reason": "left_service"}),
        });
        assert_eq!(events, expected);
    }
    db.finish().await
}
