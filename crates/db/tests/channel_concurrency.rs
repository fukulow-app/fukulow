mod support;
use db::PostMessage;
use std::sync::Arc;
use support::{Conversation, Database, Result, message_id, wait_for_blocked_operations};
use tokio::sync::Barrier;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adding_members_concurrently_with_departure_never_leaves_a_membership() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    for _ in 0..20 {
        let actor = db.person().await?;
        let mut blocker = db.owner.begin().await?;
        sqlx::query("SELECT id FROM actors WHERE id = $1 FOR UPDATE")
            .bind(actor.0)
            .fetch_one(&mut *blocker)
            .await?;
        let barrier = Arc::new(Barrier::new(3));
        let pool = db.app.clone();
        let add_barrier = barrier.clone();
        let (owner, org, channel) = (c.owner, c.organization, c.channel);
        let add = tokio::spawn(async move {
            add_barrier.wait().await;
            db::add_channel_member(&pool, owner, org, channel, actor).await
        });
        let pool = db.app.clone();
        let leave_barrier = barrier.clone();
        let leave = tokio::spawn(async move {
            leave_barrier.wait().await;
            db::leave_service(&pool, actor).await
        });
        barrier.wait().await;
        wait_for_blocked_operations(&db.owner, 2).await?;
        blocker.commit().await?;
        match add.await? {
            Ok(()) | Err(db::AddChannelMemberError::ActorDeparted) => {}
            Err(error) => return Err(error.into()),
        }
        leave.await??;
        let survivors: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1 AND m.actor_id = $2").bind(org.0).bind(actor.0).fetch_one(&db.app).await?;
        assert_eq!(survivors, 0);
        let departed: bool =
            sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM actors WHERE id = $1")
                .bind(actor.0)
                .fetch_one(&db.app)
                .await?;
        assert!(departed);
    }
    db.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn membership_addition_waits_for_departure_before_checking_actor() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let actor = db.person().await?;
    let mut departure = db.owner.begin().await?;
    sqlx::query("UPDATE actors SET deleted_at = now() WHERE id = $1")
        .bind(actor.0)
        .execute(&mut *departure)
        .await?;
    let pool = db.app.clone();
    let (owner, org, channel) = (c.owner, c.organization, c.channel);
    let add =
        tokio::spawn(
            async move { db::add_channel_member(&pool, owner, org, channel, actor).await },
        );
    wait_for_blocked_operations(&db.owner, 1).await?;
    departure.commit().await?;
    assert!(matches!(
        add.await?,
        Err(db::AddChannelMemberError::ActorDeparted)
    ));
    let survivors: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1 AND m.actor_id = $2").bind(org.0).bind(actor.0).fetch_one(&db.app).await?;
    assert_eq!(survivors, 0);
    db.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_posts_are_consecutive_and_concurrent_retries_are_one_send() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    for same_id in [false, true] {
        for _ in 0..10 {
            let next = c.counter(&db).await? + 1;
            let first = message_id();
            let second = if same_id { first } else { message_id() };
            let mut blocker = db.owner.begin().await?;
            sqlx::query(
                "SELECT id FROM channels WHERE organization_id = $1 AND id = $2 FOR UPDATE",
            )
            .bind(c.organization.0)
            .bind(c.channel.0)
            .fetch_one(&mut *blocker)
            .await?;
            let barrier = Arc::new(Barrier::new(3));
            let mut tasks = Vec::new();
            for id in [first, second] {
                let pool = db.app.clone();
                let barrier = barrier.clone();
                let (org, channel, sender) = (c.organization, c.channel, c.owner);
                tasks.push(tokio::spawn(async move {
                    barrier.wait().await;
                    db::post_message(&pool, org, channel, sender, id, "concurrent body").await
                }));
            }
            barrier.wait().await;
            wait_for_blocked_operations(&db.owner, 2).await?;
            blocker.commit().await?;
            let mut posted = Vec::new();
            let mut retried = Vec::new();
            for task in tasks {
                match task.await?? {
                    PostMessage::Posted(message) => posted.push(message),
                    PostMessage::AlreadyPosted(message) => retried.push(message),
                }
            }
            posted.sort_by_key(|m| m.channel_seq);
            if same_id {
                assert_eq!(posted.len(), 1);
                assert_eq!(retried, posted);
                assert_eq!(posted[0].channel_seq.get(), next);
            } else {
                assert!(retried.is_empty());
                assert_eq!(
                    posted
                        .iter()
                        .map(|m| m.channel_seq.get())
                        .collect::<Vec<_>>(),
                    [next, next + 1]
                );
            }
            assert_eq!(c.counter(&db).await?, next + i64::from(!same_id));
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM messages WHERE organization_id = $1 AND channel_id = $2",
            )
            .bind(c.organization.0)
            .bind(c.channel.0)
            .fetch_one(&db.app)
            .await?;
            assert_eq!(count, c.counter(&db).await?);
        }
    }
    db.finish().await
}
