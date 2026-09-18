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

use domain::{InviteId, OrganizationId, OrganizationRole};
use std::time::Duration;
use support::{Database, Result, invites as fixtures, wait_for_blocked_operations};
use time::OffsetDateTime;
use uuid::Uuid;

fn fixture_hash() -> db::TokenHash {
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(Uuid::now_v7().as_bytes());
    db::TokenHash::from_bytes(bytes)
}

#[tokio::test]
async fn invite_mutations_authorize_active_membership_in_the_named_organization() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let org = db.organization(owner).await?;
    let actor = db.person().await?;
    db.member(org, actor, "admin").await?;
    let other = db.organization(actor).await?;
    let outsider = db.person().await?;
    for (acting, target, role, status, expected) in [
        (actor, org, "owner", "active", "allowed"),
        (actor, org, "admin", "active", "allowed"),
        (actor, org, "member", "active", "forbidden"),
        (actor, org, "owner", "suspended", "not_member"),
        (actor, org, "admin", "left", "not_member"),
        (outsider, org, "admin", "active", "not_member"),
        (owner, other, "admin", "active", "not_member"),
        (
            actor,
            OrganizationId(Uuid::now_v7()),
            "admin",
            "active",
            "not_member",
        ),
    ] {
        fixtures::set_role(&db, org, actor, role, status).await?;
        let id = fixtures::invite(&db, owner, org, &fixture_hash(), 1).await?;
        let before = fixtures::counts(&db).await?;
        let row = fixtures::stored(&db, org, id).await?;
        let create = db::create_invite(
            &db.app,
            acting,
            target,
            fixtures::token(&fixture_hash()),
            OffsetDateTime::now_utc() + time::Duration::hours(1),
            1,
        )
        .await;
        let revoke = db::revoke_invite(&db.app, acting, target, Some(id)).await;
        match expected {
            "allowed" => {
                create?;
                revoke?;
                assert!(!fixtures::stored(&db, org, id).await?["revoked_at"].is_null());
            }
            "forbidden" => {
                assert!(matches!(create, Err(db::CreateInviteError::Forbidden)));
                assert!(matches!(revoke, Err(db::RevokeInviteError::Forbidden)));
            }
            _ => {
                assert!(matches!(create, Err(db::CreateInviteError::NotAMember)));
                assert!(matches!(revoke, Err(db::RevokeInviteError::NotAMember)));
            }
        }
        if expected != "allowed" {
            assert_eq!(fixtures::counts(&db).await?, before);
            assert_eq!(fixtures::stored(&db, org, id).await?, row);
        }
    }
    db.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_waits_for_uncommitted_demotion_and_refuses_without_writes() -> Result {
    pending_demotion(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_waits_for_uncommitted_demotion_and_refuses_without_writes() -> Result {
    pending_demotion(true).await
}

async fn pending_demotion(revoke: bool) -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let org = db.organization(owner).await?;
    let actor = db.person().await?;
    db.member(org, actor, "admin").await?;
    let id = fixtures::invite(&db, owner, org, &fixture_hash(), 1).await?;
    let before = fixtures::counts(&db).await?;
    let row = fixtures::stored(&db, org, id).await?;
    let mut demotion = db.actor(owner).await?;
    // Only the membership lock is held so this test isolates authorize's FOR SHARE.
    sqlx::query("UPDATE organization_members SET role = 'member' WHERE organization_id = $1 AND actor_id = $2")
        .bind(org.0).bind(actor.0).execute(&mut *demotion).await?;
    let pool = db.app.clone();
    let mut task = tokio::spawn(async move {
        if revoke {
            matches!(
                db::revoke_invite(&pool, actor, org, Some(id)).await,
                Err(db::RevokeInviteError::Forbidden)
            )
        } else {
            matches!(
                db::create_invite(
                    &pool,
                    actor,
                    org,
                    fixtures::token(&fixture_hash()),
                    OffsetDateTime::now_utc() + time::Duration::hours(1),
                    1
                )
                .await,
                Err(db::CreateInviteError::Forbidden)
            )
        }
    });
    let early = tokio::select! {
        result = &mut task => Some(result?),
        blocked = wait_for_blocked_operations(&db.owner, 1) => { blocked?; None }
    };
    let was_pending = early.is_none() && !task.is_finished();
    demotion.commit().await?;
    let forbidden = match early {
        Some(result) => result,
        None => tokio::time::timeout(Duration::from_secs(5), task).await??,
    };
    assert!(
        was_pending,
        "invite operation completed before demotion committed"
    );
    assert!(
        forbidden,
        "invite operation did not read the committed demotion"
    );
    assert_eq!(fixtures::counts(&db).await?, before);
    assert_eq!(fixtures::stored(&db, org, id).await?, row);
    db.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invite_creation_and_role_change_do_not_deadlock() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let org = db.organization(owner).await?;
    let actor = db.person().await?;
    db.member(org, actor, "admin").await?;
    for _ in 0..20 {
        fixtures::set_role(&db, org, actor, "admin", "active").await?;
        let mut blocker = db.inspector.begin().await?;
        sqlx::query("SELECT id FROM organizations WHERE id = $1 FOR UPDATE")
            .bind(org.0)
            .fetch_one(&mut *blocker)
            .await?;
        let pool = db.app.clone();
        let change = tokio::spawn(async move {
            db::change_member_role(&pool, actor, org, actor, OrganizationRole::Member).await
        });
        wait_for_blocked_operations(&db.owner, 1).await?;
        let pool = db.app.clone();
        let create = tokio::spawn(async move {
            db::create_invite(
                &pool,
                actor,
                org,
                fixtures::token(&fixture_hash()),
                OffsetDateTime::now_utc() + time::Duration::hours(1),
                1,
            )
            .await
        });
        // Queue the demotion first. A membership-first invite would now hold the
        // row it needs, and releasing the organization would complete the cycle.
        wait_for_blocked_operations(&db.owner, 2).await?;
        assert!(!change.is_finished() && !create.is_finished());
        blocker.commit().await?;
        let (change, create) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(change, create)
        })
        .await?;
        let change = change?;
        let create = create?;
        if let Err(db::ChangeMemberRoleError::Database(error)) = &change {
            assert_ne!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("40P01")
            );
        }
        if let Err(db::CreateInviteError::Database(error)) = &create {
            assert_ne!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("40P01")
            );
        }
        change?;
        assert!(matches!(create, Err(db::CreateInviteError::Forbidden)));
    }
    db.finish().await
}

#[tokio::test]
async fn authorization_precedes_deferred_input_errors_and_missing_invites() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let org = db.organization(owner).await?;
    let actor = db.person().await?;
    db.member(org, actor, "member").await?;
    assert!(matches!(
        db::create_invite(
            &db.app,
            actor,
            org,
            || None::<((), db::TokenHash)>,
            OffsetDateTime::now_utc(),
            1
        )
        .await,
        Err(db::CreateInviteError::Forbidden)
    ));
    for invite in [None, Some(InviteId(Uuid::now_v7()))] {
        assert!(matches!(
            db::revoke_invite(&db.app, actor, org, invite).await,
            Err(db::RevokeInviteError::Forbidden)
        ));
    }
    assert!(matches!(
        db::revoke_invite(&db.app, owner, org, None).await,
        Err(db::RevokeInviteError::NotFound)
    ));
    db.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn acceptance_and_revocation_do_not_deadlock() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let org = db.organization(owner).await?;
    let hash = fixture_hash();
    let id = fixtures::invite(&db, owner, org, &hash, 1).await?;
    let mut blocker = db.app.begin().await?;
    db::create_person(
        &mut blocker,
        "pending-invite@example.invalid",
        support::PASSWORD_HASH,
        "Fixture joiner",
    )
    .await?;
    let pool = db.app.clone();
    let accept = tokio::spawn(async move {
        db::accept_invite(
            &pool,
            &hash,
            "pending-invite@example.invalid",
            support::PASSWORD_HASH,
            "Fixture joiner",
        )
        .await
    });
    // The pending unique email holds acceptance after its invite UPDATE but before
    // the membership INSERT takes the organization's foreign-key lock.
    wait_for_blocked_operations(&db.owner, 1).await?;
    let pool = db.app.clone();
    let revoke = tokio::spawn(async move { db::revoke_invite(&pool, owner, org, Some(id)).await });
    wait_for_blocked_operations(&db.owner, 2).await?;
    blocker.rollback().await?;
    let (accept, revoke) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(accept, revoke)
    })
    .await?;
    let accept = accept?;
    let revoke = revoke?;
    if let Err(db::AcceptInviteError::Database(error)) = &accept {
        assert_ne!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("40P01")
        );
    }
    if let Err(db::RevokeInviteError::Database(error)) = &revoke {
        assert_ne!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("40P01")
        );
    }
    accept?;
    revoke?;
    let stored = fixtures::stored(&db, org, id).await?;
    assert_eq!(stored["used_count"], 1);
    assert!(!stored["revoked_at"].is_null());
    db.finish().await
}
