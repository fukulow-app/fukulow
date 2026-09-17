mod support;
use domain::{OrganizationRole, TeamRole};
use support::{Database, Result};

#[tokio::test]
async fn change_member_role_cannot_target_another_organization() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let member = db.person().await?;
    let a = db.organization(owner).await?;
    let b = db.organization(owner).await?;
    db.member(b, member, "member").await?;
    let team = db::create_team(&db.app, owner, b, "Fixture").await?;
    db::add_team_member(&db.app, owner, b, team, member, TeamRole::Member).await?;
    let before = db.audit(b).await?;
    let state: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert!(matches!(
        db::change_member_role(&db.app, owner, a, member, OrganizationRole::Admin).await,
        Err(db::ChangeMemberRoleError::NotFound)
    ));
    let after: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(state, after);
    let role: String = sqlx::query_scalar("SELECT role FROM team_members WHERE organization_id = $1 AND team_id = $2 AND actor_id = $3").bind(b.0).bind(team.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(role, "member");
    assert_eq!(db.audit(b).await?, before);
    db.finish().await
}

#[tokio::test]
async fn suspend_member_cannot_target_another_organization() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let member = db.person().await?;
    let a = db.organization(owner).await?;
    let b = db.organization(owner).await?;
    db.member(b, member, "member").await?;
    let team = db::create_team(&db.app, owner, b, "Fixture").await?;
    db::add_team_member(&db.app, owner, b, team, member, TeamRole::Member).await?;
    let before = db.audit(b).await?;
    let state: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert!(matches!(
        db::suspend_member(&db.app, owner, a, member).await,
        Err(db::SuspendMemberError::NotFound)
    ));
    let after: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(state, after);
    let role: String = sqlx::query_scalar("SELECT role FROM team_members WHERE organization_id = $1 AND team_id = $2 AND actor_id = $3").bind(b.0).bind(team.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(role, "member");
    assert_eq!(db.audit(b).await?, before);
    db.finish().await
}

#[tokio::test]
async fn reactivate_member_cannot_target_another_organization() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let member = db.person().await?;
    let a = db.organization(owner).await?;
    let b = db.organization(owner).await?;
    db.member(b, member, "member").await?;
    let team = db::create_team(&db.app, owner, b, "Fixture").await?;
    db::add_team_member(&db.app, owner, b, team, member, TeamRole::Member).await?;
    db::suspend_member(&db.app, owner, b, member).await?;
    let before = db.audit(b).await?;
    let state: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert!(matches!(
        db::reactivate_member(&db.app, owner, a, member).await,
        Err(db::ReactivateMemberError::NotFound)
    ));
    let after: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(state, after);
    let role: String = sqlx::query_scalar("SELECT role FROM team_members WHERE organization_id = $1 AND team_id = $2 AND actor_id = $3").bind(b.0).bind(team.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(role, "member");
    assert_eq!(db.audit(b).await?, before);
    db.finish().await
}

#[tokio::test]
async fn add_team_member_cannot_target_another_organization() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let member = db.person().await?;
    let a = db.organization(owner).await?;
    let b = db.organization(owner).await?;
    db.member(b, member, "member").await?;
    let team = db::create_team(&db.app, owner, b, "Fixture").await?;
    db::add_team_member(&db.app, owner, b, team, member, TeamRole::Member).await?;
    let before = db.audit(b).await?;
    let state: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert!(matches!(
        db::add_team_member(&db.app, owner, a, team, member, TeamRole::Member).await,
        Err(db::AddTeamMemberError::NotFound)
    ));
    let after: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(state, after);
    let role: String = sqlx::query_scalar("SELECT role FROM team_members WHERE organization_id = $1 AND team_id = $2 AND actor_id = $3").bind(b.0).bind(team.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(role, "member");
    assert_eq!(db.audit(b).await?, before);
    db.finish().await
}

#[tokio::test]
async fn change_team_member_role_cannot_target_another_organization() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let member = db.person().await?;
    let a = db.organization(owner).await?;
    let b = db.organization(owner).await?;
    db.member(b, member, "member").await?;
    let team = db::create_team(&db.app, owner, b, "Fixture").await?;
    db::add_team_member(&db.app, owner, b, team, member, TeamRole::Member).await?;
    let before = db.audit(b).await?;
    let state: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert!(matches!(
        db::change_team_member_role(&db.app, owner, a, team, member, TeamRole::Manager).await,
        Err(db::ChangeTeamMemberRoleError::NotFound)
    ));
    let after: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(state, after);
    let role: String = sqlx::query_scalar("SELECT role FROM team_members WHERE organization_id = $1 AND team_id = $2 AND actor_id = $3").bind(b.0).bind(team.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(role, "member");
    assert_eq!(db.audit(b).await?, before);
    db.finish().await
}

#[tokio::test]
async fn remove_team_member_cannot_target_another_organization() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let member = db.person().await?;
    let a = db.organization(owner).await?;
    let b = db.organization(owner).await?;
    db.member(b, member, "member").await?;
    let team = db::create_team(&db.app, owner, b, "Fixture").await?;
    db::add_team_member(&db.app, owner, b, team, member, TeamRole::Member).await?;
    let before = db.audit(b).await?;
    let state: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert!(matches!(
        db::remove_team_member(&db.app, owner, a, team, member).await,
        Err(db::RemoveTeamMemberError::NotFound)
    ));
    let after: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(b.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(state, after);
    let role: String = sqlx::query_scalar("SELECT role FROM team_members WHERE organization_id = $1 AND team_id = $2 AND actor_id = $3").bind(b.0).bind(team.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(role, "member");
    assert_eq!(db.audit(b).await?, before);
    db.finish().await
}
