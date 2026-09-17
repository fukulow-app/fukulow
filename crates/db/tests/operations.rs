mod support;
use domain::{ActorId, OrganizationId, OrganizationRole as Role, TeamRole};
use serde_json::json;
use support::{Audit, Database, PASSWORD_HASH, Result, rejected};
use uuid::Uuid;

fn event(
    actor: ActorId,
    action: &str,
    target_type: &str,
    target: Uuid,
    metadata: serde_json::Value,
) -> Audit {
    Audit {
        actor: actor.0,
        action: action.into(),
        target_type: target_type.into(),
        target,
        metadata,
    }
}

#[tokio::test]
async fn operations_write_exact_typed_audits_and_noop_writes_none() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let member = db.person().await?;
    let org = db.organization(owner).await?;
    db.member(org, member, "member").await?;
    db::change_member_role(&db.app, owner, org, member, Role::Admin).await?;
    db::change_member_role(&db.app, owner, org, member, Role::Admin).await?;
    db::suspend_member(&db.app, owner, org, member).await?;
    let row: (String, String, Option<String>) = sqlx::query_as("SELECT role, status, display_name FROM organization_members WHERE organization_id = $1 AND actor_id = $2")
        .bind(org.0).bind(member.0).fetch_one(&db.inspector).await?;
    assert_eq!(
        row,
        (
            "admin".into(),
            "suspended".into(),
            Some("Fixture override".into())
        )
    );
    assert!(matches!(
        db::suspend_member(&db.app, owner, org, member).await,
        Err(db::SuspendMemberError::NotActive)
    ));
    db::reactivate_member(&db.app, owner, org, member).await?;
    assert!(matches!(
        db::reactivate_member(&db.app, owner, org, member).await,
        Err(db::ReactivateMemberError::NotSuspended)
    ));
    let team = db::create_team(&db.app, owner, org, "Fixture team").await?;
    db::add_team_member(&db.app, owner, org, team, member, TeamRole::Member).await?;
    assert!(matches!(
        db::add_team_member(&db.app, owner, org, team, member, TeamRole::Member).await,
        Err(db::AddTeamMemberError::AlreadyMember)
    ));
    db::change_team_member_role(&db.app, owner, org, team, member, TeamRole::Manager).await?;
    db::change_team_member_role(&db.app, owner, org, team, member, TeamRole::Manager).await?;
    db::remove_team_member(&db.app, owner, org, team, member).await?;
    let expected = expected_audits(owner, member, org, team);
    assert_eq!(db.audit(org).await?, expected);
    db.finish().await
}

#[tokio::test]
async fn failed_audit_insert_rolls_back_the_state_change() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let member = db.person().await?;
    let org = db.organization(owner).await?;
    db.member(org, member, "member").await?;
    let before = db.audit(org).await?;
    sqlx::query("ALTER TABLE audit_events ADD CONSTRAINT reject_audit CHECK (false) NOT VALID")
        .execute(&db.owner)
        .await?;
    let result = db::change_member_role(&db.app, owner, org, member, Role::Admin).await;
    assert!(matches!(
        result,
        Err(db::ChangeMemberRoleError::Database(_))
    ));
    let role: String = sqlx::query_scalar(
        "SELECT role FROM organization_members WHERE organization_id = $1 AND actor_id = $2",
    )
    .bind(org.0)
    .bind(member.0)
    .fetch_one(&db.inspector)
    .await?;
    assert_eq!(role, "member");
    assert_eq!(db.audit(org).await?, before);
    db.finish().await
}

#[tokio::test]
async fn organization_and_first_owner_are_atomic() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    sqlx::query("ALTER TABLE audit_events ADD CONSTRAINT reject_audit CHECK (false) NOT VALID")
        .execute(&db.owner)
        .await?;
    assert!(matches!(
        db::create_organization(&db.app, owner, "Fixture organization", "fixture").await,
        Err(db::CreateOrganizationError::Database(_))
    ));
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM organizations WHERE slug = 'fixture')")
            .fetch_one(&db.inspector)
            .await?;
    assert!(!exists);
    sqlx::query("ALTER TABLE audit_events DROP CONSTRAINT reject_audit")
        .execute(&db.owner)
        .await?;
    let org = db.organization(owner).await?;
    let membership: (String, String) = sqlx::query_as("SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2").bind(org.0).bind(owner.0).fetch_one(&db.inspector).await?;
    assert_eq!(membership, ("owner".into(), "active".into()));
    db.finish().await
}

#[tokio::test]
async fn create_person_maps_email_conflicts_and_cleans_up_the_actor() -> Result {
    let db = Database::new().await?;
    let mut conn = db.app.begin().await?;
    let actor = db::create_person(
        &mut conn,
        "fixture@example.invalid",
        PASSWORD_HASH,
        "Fixture person",
    )
    .await?;
    assert_eq!(actor.0.get_version_num(), 7);
    assert!(matches!(
        db::create_person(
            &mut conn,
            "FIXTURE@example.invalid",
            PASSWORD_HASH,
            "Other fixture"
        )
        .await,
        Err(db::CreatePersonError::EmailTaken)
    ));
    conn.commit().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM actors")
        .fetch_one(&db.inspector)
        .await?;
    assert_eq!(count, 1);
    db.finish().await
}

#[tokio::test]
async fn create_organization_maps_its_domain_errors() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    assert!(matches!(
        db::create_organization(&db.app, actor, "Fixture", "Bad_slug").await,
        Err(db::CreateOrganizationError::InvalidSlug)
    ));
    db::create_organization(&db.app, actor, "Fixture", "fixture").await?;
    assert!(matches!(
        db::create_organization(&db.app, actor, "Fixture", "fixture").await,
        Err(db::CreateOrganizationError::SlugTaken)
    ));
    assert!(matches!(
        db::create_organization(&db.app, ActorId(Uuid::now_v7()), "Fixture", "other").await,
        Err(db::CreateOrganizationError::ActorNotFound)
    ));
    db.finish().await
}

#[tokio::test]
async fn last_owner_cannot_be_demoted_suspended_or_depart() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let org = db.organization(owner).await?;
    assert!(matches!(
        db::change_member_role(&db.app, owner, org, owner, Role::Member).await,
        Err(db::ChangeMemberRoleError::LastOwner)
    ));
    assert!(matches!(
        db::suspend_member(&db.app, owner, org, owner).await,
        Err(db::SuspendMemberError::LastOwner)
    ));
    assert!(
        matches!(db::leave_service(&db.app, owner).await, Err(db::LeaveServiceError::LastOwner {organization_id}) if organization_id == org)
    );
    assert_eq!(db.audit(org).await?.len(), 2);
    db.finish().await
}

#[tokio::test]
async fn create_team_is_scoped_and_missing_organization_is_not_found() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let org = db.organization(owner).await?;
    let other = db.organization(owner).await?;
    assert!(matches!(
        db::create_team(&db.app, owner, OrganizationId(Uuid::now_v7()), "Fixture").await,
        Err(db::CreateTeamError::NotFound)
    ));
    let team = db::create_team(&db.app, owner, org, "Fixture").await?;
    let own: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM teams WHERE organization_id = $1 AND id = $2)",
    )
    .bind(org.0)
    .bind(team.0)
    .fetch_one(&db.inspector)
    .await?;
    let foreign: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM teams WHERE organization_id = $1 AND id = $2)",
    )
    .bind(other.0)
    .bind(team.0)
    .fetch_one(&db.inspector)
    .await?;
    assert!(own && !foreign);
    db.finish().await
}

#[tokio::test]
async fn audit_update_is_denied_to_the_application() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    let org = db.organization(actor).await?;
    rejected(
        sqlx::query("UPDATE audit_events SET metadata = '{}' WHERE organization_id = $1")
            .bind(org.0)
            .execute(&db.app)
            .await,
        "42501",
    );
    db.finish().await
}

#[tokio::test]
async fn audit_delete_is_denied_to_the_application() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    let org = db.organization(actor).await?;
    rejected(
        sqlx::query("DELETE FROM audit_events WHERE organization_id = $1")
            .bind(org.0)
            .execute(&db.app)
            .await,
        "42501",
    );
    db.finish().await
}

#[tokio::test]
async fn actor_type_is_immutable_before_and_after_departure_and_for_bots() -> Result {
    let db = Database::new().await?;
    let person = db.person().await?;
    rejected(
        sqlx::query("UPDATE actors SET type = 'system' WHERE id = $1")
            .bind(person.0)
            .execute(&db.app)
            .await,
        "42501",
    );
    db::leave_service(&db.app, person).await?;
    rejected(
        sqlx::query("UPDATE actors SET type = 'system' WHERE id = $1")
            .bind(person.0)
            .execute(&db.app)
            .await,
        "42501",
    );
    let bot = Uuid::now_v7();
    sqlx::query("INSERT INTO actors (id, type) VALUES ($1, 'bot')")
        .bind(bot)
        .execute(&db.app)
        .await?;
    rejected(
        sqlx::query("UPDATE actors SET type = 'human' WHERE id = $1")
            .bind(bot)
            .execute(&db.app)
            .await,
        "42501",
    );
    db.finish().await
}

#[tokio::test]
async fn identity_parent_and_kind_columns_are_immutable() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    let org = db.organization(actor).await?;
    let other = db.organization(actor).await?;
    let team = db::create_team(&db.app, actor, org, "Fixture").await?;
    rejected(sqlx::query("UPDATE organization_members SET organization_id = $3 WHERE organization_id = $1 AND actor_id = $2").bind(org.0).bind(actor.0).bind(other.0).execute(&db.app).await, "42501");
    rejected(sqlx::query("UPDATE organization_members SET actor_id = $3 WHERE organization_id = $1 AND actor_id = $2").bind(org.0).bind(actor.0).bind(Uuid::now_v7()).execute(&db.app).await, "42501");
    rejected(
        sqlx::query("UPDATE teams SET organization_id = $3 WHERE organization_id = $1 AND id = $2")
            .bind(org.0)
            .bind(team.0)
            .bind(other.0)
            .execute(&db.app)
            .await,
        "42501",
    );
    rejected(
        sqlx::query("UPDATE invites SET kind = 'team' WHERE organization_id = $1")
            .bind(org.0)
            .execute(&db.app)
            .await,
        "42501",
    );
    db.finish().await
}

#[tokio::test]
async fn database_errors_do_not_format_personal_data_or_expose_a_source() -> Result {
    use std::error::Error;
    let db = Database::new().await?;
    let error = sqlx::query("SELECT 'fixture@example.invalid'::integer")
        .execute(&db.app)
        .await
        .err()
        .ok_or("expected a database error")?;
    let error = db::CreatePersonError::Database(error);
    assert!(!format!("{error} {error:?}").contains("fixture@example.invalid"));
    assert!(error.source().is_none());
    db.finish().await
}

fn expected_audits(
    owner: ActorId,
    member: ActorId,
    org: OrganizationId,
    team: domain::TeamId,
) -> Vec<Audit> {
    vec![
        event(
            owner,
            "organization.created",
            "organization",
            org.0,
            json!({}),
        ),
        event(
            owner,
            "organization.member.added",
            "actor",
            owner.0,
            json!({"role":"owner"}),
        ),
        event(
            owner,
            "organization.member.reactivated",
            "actor",
            member.0,
            json!({}),
        ),
        event(
            owner,
            "organization.member.role_changed",
            "actor",
            member.0,
            json!({"from":"member","to":"admin"}),
        ),
        event(
            owner,
            "organization.member.suspended",
            "actor",
            member.0,
            json!({}),
        ),
        event(owner, "team.created", "team", team.0, json!({})),
        event(
            owner,
            "team.member.added",
            "actor",
            member.0,
            json!({"team_id":team,"role":"member"}),
        ),
        event(
            owner,
            "team.member.removed",
            "actor",
            member.0,
            json!({"team_id":team,"reason":"removed"}),
        ),
        event(
            owner,
            "team.member.role_changed",
            "actor",
            member.0,
            json!({"team_id":team,"from":"member","to":"manager"}),
        ),
    ]
}
