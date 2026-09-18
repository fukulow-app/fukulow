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
use db::PostMessage;
use domain::{ChannelId, ChannelScope, ChannelSeq};
use support::{Conversation, Database, Result, message_id};
use uuid::Uuid;

#[tokio::test]
async fn posting_refuses_foreign_and_missing_channels_without_consuming_a_number() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    let before = db.audit(a.organization).await?;
    for channel in [b.channel, ChannelId(Uuid::now_v7())] {
        assert!(matches!(
            db::post_message(
                &db.app,
                a.organization,
                channel,
                a.owner,
                message_id(),
                "body"
            )
            .await,
            Err(db::PostMessageError::NotFound)
        ));
    }
    assert_eq!(b.counter(&db).await?, 0);
    assert_eq!(a.counter(&db).await?, 0);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM messages WHERE organization_id = $1 OR organization_id = $2",
    )
    .bind(a.organization.0)
    .bind(b.organization.0)
    .fetch_one(&db.inspector)
    .await?;
    assert_eq!(count, 0);
    assert_eq!(db.audit(a.organization).await?, before);
    db.finish().await
}

#[tokio::test]
async fn messages_before_refuses_foreign_and_missing_channels() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    b.post(&db).await?;
    for channel in [b.channel, ChannelId(Uuid::now_v7())] {
        assert!(matches!(
            db::messages_before(&db.app, a.owner, a.organization, channel, None, 100).await,
            Err(db::MessagesBeforeError::NotFound)
        ));
    }
    assert_eq!(b.counter(&db).await?, 1);
    db.finish().await
}

#[tokio::test]
async fn messages_after_refuses_foreign_and_missing_channels() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    b.post(&db).await?;
    for channel in [b.channel, ChannelId(Uuid::now_v7())] {
        assert!(matches!(
            db::messages_after(
                &db.app,
                a.owner,
                a.organization,
                channel,
                ChannelSeq::BEGINNING,
                100
            )
            .await,
            Err(db::MessagesAfterError::NotFound)
        ));
    }
    assert_eq!(b.counter(&db).await?, 1);
    db.finish().await
}

fn sequences(messages: Vec<db::Message>) -> Vec<i64> {
    messages
        .into_iter()
        .map(|message| message.channel_seq.get())
        .collect()
}

#[tokio::test]
async fn paging_is_exclusive_ordered_clamped_and_empty_at_both_ends() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let audit = db.audit(c.organization).await?;
    assert!(
        db::messages_before(&db.app, c.owner, c.organization, c.channel, None, 100)
            .await?
            .is_empty()
    );
    assert!(
        db::messages_after(
            &db.app,
            c.owner,
            c.organization,
            c.channel,
            ChannelSeq::BEGINNING,
            100
        )
        .await?
        .is_empty()
    );
    for n in 1..=250 {
        assert_eq!(c.post(&db).await?.channel_seq.get(), n);
    }
    for (before, expected) in [
        (None, (151..=250).rev().collect::<Vec<_>>()),
        (Some(151), (51..=150).rev().collect()),
        (Some(51), (1..=50).rev().collect()),
        (Some(1), vec![]),
    ] {
        let page = db::messages_before(
            &db.app,
            c.owner,
            c.organization,
            c.channel,
            before.map(|n| ChannelSeq::new(n).unwrap()),
            100,
        )
        .await?;
        assert_eq!(sequences(page), expected);
    }
    for (after, expected) in [
        (ChannelSeq::BEGINNING, (1..=100).collect::<Vec<_>>()),
        (ChannelSeq::new(100)?, (101..=200).collect()),
        (ChannelSeq::new(250)?, vec![]),
    ] {
        assert_eq!(
            sequences(
                db::messages_after(&db.app, c.owner, c.organization, c.channel, after, 100).await?
            ),
            expected
        );
    }
    for (limit, expected) in [(0, 1), (500, 200)] {
        assert_eq!(
            db::messages_before(&db.app, c.owner, c.organization, c.channel, None, limit)
                .await?
                .len(),
            expected
        );
        assert_eq!(
            db::messages_after(
                &db.app,
                c.owner,
                c.organization,
                c.channel,
                ChannelSeq::BEGINNING,
                limit
            )
            .await?
            .len(),
            expected
        );
    }
    assert_eq!(db.audit(c.organization).await?, audit);
    db.finish().await
}

#[tokio::test]
async fn failed_insert_rolls_back_the_counter() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    sqlx::query("ALTER TABLE messages ADD CONSTRAINT reject_message CHECK (false) NOT VALID")
        .execute(&db.owner)
        .await?;
    assert!(matches!(
        db::post_message(
            &db.app,
            c.organization,
            c.channel,
            c.owner,
            message_id(),
            "body"
        )
        .await,
        Err(db::PostMessageError::Database(_))
    ));
    assert_eq!(c.counter(&db).await?, 0);
    sqlx::query("ALTER TABLE messages DROP CONSTRAINT reject_message")
        .execute(&db.owner)
        .await?;
    assert_eq!(c.post(&db).await?.channel_seq.get(), 1);
    db.finish().await
}

#[tokio::test]
async fn retry_returns_stored_message_and_does_not_consume_a_number() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let original = c.post(&db).await?;
    let retry = db::post_message(
        &db.app,
        c.organization,
        c.channel,
        c.owner,
        original.id,
        "Changed retry body",
    )
    .await?;
    assert_eq!(retry, PostMessage::AlreadyPosted(original));
    assert_eq!(c.counter(&db).await?, 1);
    assert_eq!(c.post(&db).await?.channel_seq.get(), 2);
    assert_eq!(
        db::messages_before(&db.app, c.owner, c.organization, c.channel, None, 100)
            .await?
            .len(),
        2
    );
    db.finish().await
}

#[tokio::test]
async fn id_conflicts_do_not_disclose_another_sender_channel_or_organization() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    db.member(a.organization, b.owner, "member").await?;
    let original = a.post(&db).await?;
    let other = db::create_channel(
        &db.app,
        a.owner,
        a.organization,
        ChannelScope::Organization,
        "general",
    )
    .await?;
    for (org, channel, sender) in [
        (a.organization, a.channel, b.owner),
        (a.organization, other, a.owner),
        (b.organization, b.channel, b.owner),
    ] {
        let error = db::post_message(&db.app, org, channel, sender, original.id, "different")
            .await
            .unwrap_err();
        assert!(matches!(error, db::PostMessageError::IdConflict));
        assert_eq!(error.to_string(), "message id conflict");
    }
    assert_eq!(a.counter(&db).await?, 1);
    assert_eq!(b.counter(&db).await?, 0);
    assert!(
        db::messages_before(&db.app, a.owner, a.organization, other, None, 100)
            .await?
            .is_empty()
    );
    db.finish().await
}

#[tokio::test]
async fn invalid_body_is_refused_before_connecting_and_unicode_counts_as_characters() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let closed = db.app.clone();
    // A separate closed pool proves validation occurs before beginning a transaction.
    let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy_with(Default::default());
    pool.close().await;
    for body in [String::new(), "x".repeat(4001)] {
        assert!(matches!(
            db::post_message(
                &pool,
                c.organization,
                c.channel,
                c.owner,
                message_id(),
                &body
            )
            .await,
            Err(db::PostMessageError::InvalidBody)
        ));
    }
    let result = db::post_message(
        &closed,
        c.organization,
        c.channel,
        c.owner,
        message_id(),
        &"界".repeat(4000),
    )
    .await?;
    assert!(matches!(result, PostMessage::Posted(_)));
    assert_eq!(c.counter(&db).await?, 1);
    db.finish().await
}
