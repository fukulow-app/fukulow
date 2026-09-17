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
        .fetch_one(&db.inspector)
        .await?;
    assert_eq!(count, 1);
    db.finish().await
}

#[tokio::test]
async fn organization_member_can_join_and_only_success_is_audited() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let outside = db.person().await?;
    db.member(c.organization, outside, "member").await?;
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
        Err(db::AddChannelMemberError::NotFound)
    ));
    let departed = db.person().await?;
    db.member(c.organization, departed, "member").await?;
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
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1").bind(c.organization.0).fetch_one(&db.inspector).await?;
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
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1 OR c.organization_id = $2").bind(a.organization.0).bind(b.organization.0).fetch_one(&db.inspector).await?;
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
    db::add_channel_member(&db.app, a.owner, a.organization, a.channel, actor).await?;
    // The application cannot list an outside actor until #43, so the fixture stores it directly.
    list_outside_actor(&db, &b, actor).await?;
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
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1 AND m.actor_id = $2").bind(c.organization.0).bind(actor.0).fetch_one(&db.inspector).await?;
        assert_eq!(count, 0);
    }
    let outsider = db.person().await?;
    list_outside_actor(&db, &b, outsider).await?;
    db::leave_service(&db.app, outsider).await?;
    let records: Vec<_> = db
        .audit(b.organization)
        .await?
        .into_iter()
        .filter(|a| a.action == "channel.member.removed" && a.target == outsider.0)
        .collect();
    assert_eq!(records.len(), 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1 AND m.actor_id = $2").bind(b.organization.0).bind(outsider.0).fetch_one(&db.inspector).await?;
    assert_eq!(count, 0);
    db.finish().await
}

#[tokio::test]
async fn an_outside_departure_audit_is_admitted_only_for_the_actors_own_listing() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    let outsider = db.person().await?;
    let stranger = db.person().await?;
    list_outside_actor(&db, &b, outsider).await?;
    list_outside_actor(&db, &a, stranger).await?;
    let valid = (
        b.organization.0,
        outsider.0,
        "channel.member.removed",
        b.channel.0,
        "left_service",
    );
    for (organization, target, action, channel, reason) in [
        (
            a.organization.0,
            outsider.0,
            "channel.member.removed",
            b.channel.0,
            "left_service",
        ),
        (
            b.organization.0,
            outsider.0,
            "channel.member.removed",
            a.channel.0,
            "left_service",
        ),
        (
            b.organization.0,
            outsider.0,
            "channel.member.removed",
            b.channel.0,
            "removed",
        ),
        (
            b.organization.0,
            outsider.0,
            "channel.member.added",
            b.channel.0,
            "left_service",
        ),
        (
            b.organization.0,
            stranger.0,
            "channel.member.removed",
            b.channel.0,
            "left_service",
        ),
        valid,
    ] {
        let mut tx = db.actor(outsider).await?;
        let result = sqlx::query("INSERT INTO audit_events (id, organization_id, actor_id, action, target_type, target_id, metadata) VALUES ($1, $2, $3, $4, 'actor', $5, jsonb_build_object('channel_id', $6::uuid, 'reason', $7::text))")
            .bind(Uuid::now_v7()).bind(organization).bind(outsider.0).bind(action).bind(target).bind(channel).bind(reason)
            .execute(&mut *tx).await;
        if (organization, target, action, channel, reason) == valid {
            result?;
        } else {
            support::rejected(result, "42501");
        }
        tx.rollback().await?;
    }
    // Once the listing is gone, the same record is refused.
    let mut tx = db.actor(outsider).await?;
    sqlx::query("DELETE FROM channel_members WHERE actor_id = $1")
        .bind(outsider.0)
        .execute(&mut *tx)
        .await?;
    support::rejected(
        sqlx::query("INSERT INTO audit_events (id, organization_id, actor_id, action, target_type, target_id, metadata) VALUES ($1, $2, $3, 'channel.member.removed', 'actor', $3, jsonb_build_object('channel_id', $4::uuid, 'reason', 'left_service'))")
            .bind(Uuid::now_v7()).bind(b.organization.0).bind(outsider.0).bind(b.channel.0)
            .execute(&mut *tx).await,
        "42501",
    );
    tx.rollback().await?;
    db.finish().await
}

async fn list_outside_actor(db: &Database, c: &Conversation, actor: ActorId) -> Result {
    sqlx::query("INSERT INTO channel_members (channel_id, channel_scope, organization_id, actor_id) VALUES ($1, 'team', $2, $3)")
        .bind(c.channel.0)
        .bind(c.organization.0)
        .bind(actor.0)
        .execute(&db.inspector)
        .await?;
    Ok(())
}

#[tokio::test]
async fn channel_mutations_roll_back_when_audit_insert_fails() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let outside = db.person().await?;
    db.member(c.organization, outside, "member").await?;
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
        .fetch_one(&db.inspector)
        .await?;
    assert_eq!(count, 1);
    let members: Vec<Uuid> = sqlx::query_scalar("SELECT m.actor_id FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1").bind(c.organization.0).fetch_all(&db.inspector).await?;
    assert_eq!(members, [outside.0]);
    db.finish().await
}
