mod support;
use domain::{ActorId, ChannelId, ChannelScope, OrganizationId, TeamId};
use serde_json::json;
use support::{Audit, Conversation, Database, Result};
use uuid::Uuid;

#[tokio::test]
async fn create_channel_audits_each_scope_and_refuses_duplicate_names() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let org_channel = db::create_channel(
        &db.app,
        c.owner,
        c.organization,
        ChannelScope::Organization,
        "general",
    )
    .await?;
    for scope in [ChannelScope::Organization, ChannelScope::Team(c.team)] {
        assert!(matches!(
            db::create_channel(&db.app, c.owner, c.organization, scope, "GENERAL").await,
            Err(db::CreateChannelError::NameTaken)
        ));
    }
    let records: Vec<_> = db
        .audit(c.organization)
        .await?
        .into_iter()
        .filter(|a| a.action == "channel.created")
        .collect();
    assert_eq!(
        records,
        vec![
            Audit {
                actor: c.owner.0,
                action: "channel.created".into(),
                target_type: "channel".into(),
                target: c.channel.0,
                metadata: json!({"scope": "team", "team_id": c.team})
            },
            Audit {
                actor: c.owner.0,
                action: "channel.created".into(),
                target_type: "channel".into(),
                target: org_channel.0,
                metadata: json!({"scope": "organization"})
            },
        ]
    );
    db.finish().await
}

#[tokio::test]
async fn channel_creation_refuses_foreign_and_missing_owners_without_writes() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    let before = db.audit(a.organization).await?;
    for (org, scope) in [
        (a.organization, ChannelScope::Team(b.team)),
        (a.organization, ChannelScope::Team(TeamId(Uuid::now_v7()))),
        (OrganizationId(Uuid::now_v7()), ChannelScope::Organization),
    ] {
        assert!(matches!(
            db::create_channel(&db.app, a.owner, org, scope, "bad").await,
            Err(db::CreateChannelError::NotFound)
        ));
    }
    assert_eq!(db.audit(a.organization).await?, before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channels WHERE organization_id = $1")
        .bind(a.organization.0)
        .fetch_one(&db.app)
        .await?;
    assert_eq!(count, 1);
    db.finish().await
}

#[tokio::test]
async fn outside_actor_can_join_and_only_success_is_audited() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let outside = db.person().await?;
    db::add_channel_member(&db.app, c.owner, c.organization, c.channel, outside).await?;
    assert!(matches!(
        db::add_channel_member(&db.app, c.owner, c.organization, c.channel, outside).await,
        Err(db::AddChannelMemberError::AlreadyMember)
    ));
    let org_channel = db::create_channel(
        &db.app,
        c.owner,
        c.organization,
        ChannelScope::Organization,
        "general",
    )
    .await?;
    assert!(matches!(
        db::add_channel_member(&db.app, c.owner, c.organization, org_channel, outside).await,
        Err(db::AddChannelMemberError::OrganizationScoped)
    ));
    assert!(matches!(
        db::add_channel_member(
            &db.app,
            c.owner,
            c.organization,
            c.channel,
            ActorId(Uuid::now_v7())
        )
        .await,
        Err(db::AddChannelMemberError::ActorNotFound)
    ));
    let departed = db.person().await?;
    db::leave_service(&db.app, departed).await?;
    assert!(matches!(
        db::add_channel_member(&db.app, c.owner, c.organization, c.channel, departed).await,
        Err(db::AddChannelMemberError::ActorDeparted)
    ));
    let records: Vec<_> = db
        .audit(c.organization)
        .await?
        .into_iter()
        .filter(|a| a.action == "channel.member.added")
        .collect();
    assert_eq!(
        records,
        vec![Audit {
            actor: c.owner.0,
            action: "channel.member.added".into(),
            target_type: "actor".into(),
            target: outside.0,
            metadata: json!({"channel_id": c.channel})
        }]
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1").bind(c.organization.0).fetch_one(&db.app).await?;
    assert_eq!(count, 1);
    db.finish().await
}

#[tokio::test]
async fn adding_channel_member_refuses_foreign_and_missing_channels_without_writes() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    let before = db.audit(a.organization).await?;
    for channel in [b.channel, ChannelId(Uuid::now_v7())] {
        assert!(matches!(
            db::add_channel_member(&db.app, a.owner, a.organization, channel, a.owner).await,
            Err(db::AddChannelMemberError::NotFound)
        ));
    }
    assert_eq!(db.audit(a.organization).await?, before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1 OR c.organization_id = $2").bind(a.organization.0).bind(b.organization.0).fetch_one(&db.app).await?;
    assert_eq!(count, 0);
    db.finish().await
}

#[tokio::test]
async fn leaving_removes_internal_and_external_channels_and_audits_each_organization() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    let actor = db.person().await?;
    db.member(a.organization, actor, "member").await?;
    for c in [&a, &b] {
        db::add_channel_member(&db.app, c.owner, c.organization, c.channel, actor).await?;
    }
    db::leave_service(&db.app, actor).await?;
    for c in [&a, &b] {
        let records: Vec<_> = db
            .audit(c.organization)
            .await?
            .into_iter()
            .filter(|a| a.action == "channel.member.removed")
            .collect();
        assert_eq!(
            records,
            vec![Audit {
                actor: actor.0,
                action: "channel.member.removed".into(),
                target_type: "actor".into(),
                target: actor.0,
                metadata: json!({"channel_id": c.channel, "reason": "left_service"})
            }]
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1 AND m.actor_id = $2").bind(c.organization.0).bind(actor.0).fetch_one(&db.app).await?;
        assert_eq!(count, 0);
    }
    let outsider = db.person().await?;
    db::add_channel_member(&db.app, b.owner, b.organization, b.channel, outsider).await?;
    db::leave_service(&db.app, outsider).await?;
    let records: Vec<_> = db
        .audit(b.organization)
        .await?
        .into_iter()
        .filter(|a| a.action == "channel.member.removed" && a.target == outsider.0)
        .collect();
    assert_eq!(records.len(), 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1 AND m.actor_id = $2").bind(b.organization.0).bind(outsider.0).fetch_one(&db.app).await?;
    assert_eq!(count, 0);
    db.finish().await
}

#[tokio::test]
async fn channel_mutations_roll_back_when_audit_insert_fails() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let outside = db.person().await?;
    db::add_channel_member(&db.app, c.owner, c.organization, c.channel, outside).await?;
    let before = db.audit(c.organization).await?;
    sqlx::query("REVOKE INSERT ON audit_events FROM fukulow_app")
        .execute(&db.owner)
        .await?;
    assert!(matches!(
        db::create_channel(
            &db.app,
            c.owner,
            c.organization,
            ChannelScope::Organization,
            "failure"
        )
        .await,
        Err(db::CreateChannelError::Database(_))
    ));
    assert!(matches!(
        db::add_channel_member(&db.app, c.owner, c.organization, c.channel, c.owner).await,
        Err(db::AddChannelMemberError::Database(_))
    ));
    assert!(matches!(
        db::leave_service(&db.app, outside).await,
        Err(db::LeaveServiceError::Database(_))
    ));
    assert_eq!(db.audit(c.organization).await?, before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channels WHERE organization_id = $1")
        .bind(c.organization.0)
        .fetch_one(&db.app)
        .await?;
    assert_eq!(count, 1);
    let members: Vec<Uuid> = sqlx::query_scalar("SELECT m.actor_id FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1").bind(c.organization.0).fetch_all(&db.app).await?;
    assert_eq!(members, [outside.0]);
    db.finish().await
}
