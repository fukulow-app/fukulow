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
use domain::{ActorId, OrganizationId, OrganizationRole};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use support::{Database, Result};
use tokio::sync::Barrier;

async fn wait_for_blocked_operations(owner: &PgPool, count: i64) -> Result {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND usename = 'fukulow_app' AND cardinality(pg_blocking_pids(pid)) > 0")
                .fetch_one(owner).await?;
            if waiting >= count { return Ok::<_, sqlx::Error>(()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await??;
    Ok(())
}

async fn active_owners(pool: &PgPool, org: OrganizationId) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM organization_members WHERE organization_id = $1 AND role = 'owner' AND status = 'active'").bind(org.0).fetch_one(pool).await?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_demotions_leave_exactly_one_owner() -> Result {
    let db = Database::new().await?;
    for _ in 0..20 {
        let first = db.person().await?;
        let second = db.person().await?;
        let org = db.organization(first).await?;
        db.member(org, second, "owner").await?;
        let mut blocker = db.inspector.begin().await?;
        sqlx::query("SELECT id FROM organizations WHERE id = $1 FOR UPDATE")
            .bind(org.0)
            .fetch_one(&mut *blocker)
            .await?;
        let barrier = Arc::new(Barrier::new(3));
        let mut tasks = Vec::new();
        for actor in [first, second] {
            let pool = db.app.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                db::change_member_role(&pool, actor, org, actor, OrganizationRole::Member).await
            }));
        }
        barrier.wait().await;
        // Both transactions must have reached the lock before either may read owners.
        wait_for_blocked_operations(&db.owner, 2).await?;
        blocker.commit().await?;
        let mut successes = 0;
        for task in tasks {
            match task.await? {
                Ok(()) => successes += 1,
                Err(db::ChangeMemberRoleError::LastOwner) => {}
                Err(error) => return Err(error.into()),
            }
        }
        assert_eq!(successes, 1);
        assert_eq!(active_owners(&db.inspector, org).await?, 1);
    }
    db.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_leavers_lock_shared_organizations_in_order_without_deadlocks() -> Result {
    let db = Database::new().await?;
    for _ in 0..20 {
        let first = db.person().await?;
        let second = db.person().await?;
        let a = db.organization(first).await?;
        let b = db.organization(second).await?;
        db.member(a, second, "owner").await?;
        db.member(b, first, "owner").await?;
        let mut blocker = db.inspector.begin().await?;
        sqlx::query("SELECT id FROM organizations WHERE id = $1 FOR UPDATE")
            .bind(a.min(b).0)
            .fetch_one(&mut *blocker)
            .await?;
        let barrier = Arc::new(Barrier::new(3));
        let mut tasks = Vec::new();
        for actor in [first, second] {
            let pool = db.app.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                db::leave_service(&pool, actor).await
            }));
        }
        barrier.wait().await;
        wait_for_blocked_operations(&db.owner, 2).await?;
        blocker.commit().await?;
        let mut successes = 0;
        for task in tasks {
            match task.await? {
                Ok(()) => successes += 1,
                Err(db::LeaveServiceError::LastOwner { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        assert_eq!(successes, 1);
        assert_eq!(active_owners(&db.inspector, a).await?, 1);
        assert_eq!(active_owners(&db.inspector, b).await?, 1);
    }
    db.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn creating_an_organization_and_leaving_cannot_resurrect_an_actor() -> Result {
    let db = Database::new().await?;
    for _ in 0..20 {
        let actor = db.person().await?;
        let mut blocker = db.inspector.begin().await?;
        sqlx::query("SELECT id FROM actors WHERE id = $1 FOR UPDATE")
            .bind(actor.0)
            .fetch_one(&mut *blocker)
            .await?;
        let barrier = Arc::new(Barrier::new(3));
        let create = spawn_create(db.app.clone(), barrier.clone(), actor);
        let pool = db.app.clone();
        let leave_barrier = barrier.clone();
        let leave = tokio::spawn(async move {
            leave_barrier.wait().await;
            db::leave_service(&pool, actor).await
        });
        barrier.wait().await;
        wait_for_blocked_operations(&db.owner, 2).await?;
        blocker.commit().await?;
        match (create.await?, leave.await?) {
            (Ok(org), Err(db::LeaveServiceError::LastOwner { organization_id })) => {
                assert_eq!(org, organization_id)
            }
            (Err(db::CreateOrganizationError::ActorDeparted), Ok(())) => {}
            _ => panic!("exactly the operation committing second must be refused"),
        }
        let resurrected: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM organizations o JOIN organization_members m ON m.organization_id = o.id JOIN actors a ON a.id = m.actor_id WHERE a.id = $1 AND a.deleted_at IS NOT NULL AND m.status = 'active')")
            .bind(actor.0).fetch_one(&db.inspector).await?;
        assert!(!resurrected);
    }
    db.finish().await
}

fn spawn_create(
    pool: PgPool,
    barrier: Arc<Barrier>,
    actor: ActorId,
) -> tokio::task::JoinHandle<std::result::Result<OrganizationId, db::CreateOrganizationError>> {
    tokio::spawn(async move {
        barrier.wait().await;
        db::create_organization(
            &pool,
            actor,
            "Fixture",
            &uuid::Uuid::now_v7().simple().to_string(),
        )
        .await
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reactivation_waits_for_actor_departure_and_refuses_it() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let actor = db.person().await?;
    let org = db.organization(owner).await?;
    db.member(org, actor, "member").await?;
    db::suspend_member(&db.app, owner, org, actor).await?;
    let mut leaver = db.actor(actor).await?;
    sqlx::query("SELECT id FROM actors WHERE id = $1 FOR UPDATE")
        .bind(actor.0)
        .fetch_one(&mut *leaver)
        .await?;
    let pool = db.app.clone();
    let task = tokio::spawn(async move { db::reactivate_member(&pool, owner, org, actor).await });
    wait_for_blocked_operations(&db.owner, 1).await?;
    sqlx::query("UPDATE actors SET deleted_at = now(), display_name = NULL WHERE id = $1")
        .bind(actor.0)
        .execute(&mut *leaver)
        .await?;
    sqlx::query("UPDATE organization_members SET status = 'left', display_name = NULL WHERE organization_id = $1 AND actor_id = $2").bind(org.0).bind(actor.0).execute(&mut *leaver).await?;
    leaver.commit().await?;
    assert!(matches!(
        task.await?,
        Err(db::ReactivateMemberError::ActorDeparted)
    ));
    let status: String = sqlx::query_scalar(
        "SELECT status FROM organization_members WHERE organization_id = $1 AND actor_id = $2",
    )
    .bind(org.0)
    .bind(actor.0)
    .fetch_one(&db.inspector)
    .await?;
    assert_eq!(status, "left");
    db.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_performer_is_locked_before_the_organization() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let performer = db.person().await?;
    let member = db.person().await?;
    let org = db.organization(owner).await?;
    db.member(org, performer, "member").await?;
    db.member(org, member, "member").await?;
    let mut departure = db.actor(performer).await?;
    sqlx::query("SELECT id FROM actors WHERE id = $1 FOR UPDATE")
        .bind(performer.0)
        .fetch_one(&mut *departure)
        .await?;
    let pool = db.app.clone();
    let mutation = tokio::spawn(async move {
        db::change_member_role(&pool, performer, org, member, OrganizationRole::Admin).await
    });
    wait_for_blocked_operations(&db.owner, 1).await?;
    // If the mutator already owns the organization, its audit FK waits on the
    // actor above and this lock completes the opposite half of a deadlock.
    sqlx::query("SELECT id FROM organizations WHERE id = $1 FOR UPDATE")
        .bind(org.0)
        .fetch_one(&mut *departure)
        .await?;
    departure.commit().await?;
    mutation.await??;
    db.finish().await
}

#[tokio::test]
async fn a_role_change_waiting_on_a_departure_sees_the_membership_as_left() -> Result {
    let db = Database::new().await?;
    let owner = db.person().await?;
    let actor = db.person().await?;
    let org = db.organization(owner).await?;
    db.member(org, actor, "member").await?;
    db::suspend_member(&db.app, owner, org, actor).await?;
    // Leaving does not lock an organization where the membership is inactive. What orders the
    // two is that the departure audit's foreign key takes KEY SHARE on the organization, which
    // conflicts with change_member_role's FOR UPDATE, so the role change reads only after the
    // departure commits. Holding the membership row lines both up, the departure first.
    let mut blocker = db.inspector.begin().await?;
    sqlx::query("SELECT 1 FROM organization_members WHERE organization_id = $1 AND actor_id = $2 FOR UPDATE")
        .bind(org.0)
        .bind(actor.0)
        .fetch_one(&mut *blocker)
        .await?;
    let pool = db.app.clone();
    let leave = tokio::spawn(async move { db::leave_service(&pool, actor).await });
    wait_for_blocked_operations(&db.owner, 1).await?;
    let pool = db.app.clone();
    let change = tokio::spawn(async move {
        db::change_member_role(&pool, owner, org, actor, OrganizationRole::Admin).await
    });
    wait_for_blocked_operations(&db.owner, 2).await?;
    blocker.commit().await?;
    leave.await??;
    assert!(matches!(
        change.await?,
        Err(db::ChangeMemberRoleError::NotFound)
    ));
    let row: (String, String) = sqlx::query_as(
        "SELECT role, status FROM organization_members WHERE organization_id = $1 AND actor_id = $2",
    )
    .bind(org.0)
    .bind(actor.0)
    .fetch_one(&db.inspector)
    .await?;
    assert_eq!(row, ("member".into(), "left".into()));
    let changes: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE organization_id = $1 AND action = 'organization.member.role_changed'")
        .bind(org.0)
        .fetch_one(&db.inspector)
        .await?;
    assert_eq!(changes, 0);
    db.finish().await
}
