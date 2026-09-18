mod support;
use domain::{ActorId, ActorType};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use support::{Database, Result};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

fn hash(byte: char) -> db::TokenHash {
    db::TokenHash::from_hex(byte.to_string().repeat(64)).unwrap()
}

#[tokio::test]
async fn credential_failures_each_verify_once_and_never_insert() -> Result {
    let db = Database::new().await?;
    let email = "signin@example.invalid";
    let mut tx = db.app.begin().await?;
    let actor = db::create_person(&mut tx, email, support::PASSWORD_HASH, "Fixture person").await?;
    tx.commit().await?;
    for (address, result, expected) in [
        (email, false, support::PASSWORD_HASH),
        ("unknown@example.invalid", true, db::DUMMY_PASSWORD_HASH),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let outcome = db::sign_in(
            &db.app,
            address,
            move |phc| {
                count.fetch_add(1, Ordering::SeqCst);
                assert_eq!(phc, expected);
                result
            },
            &hash('a'),
            OffsetDateTime::now_utc() + Duration::days(14),
        )
        .await;
        assert!(matches!(outcome, Err(db::SignInError::InvalidCredentials)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    let mut tx = db.actor(actor).await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions WHERE actor_id = $1")
            .bind(actor.0)
            .fetch_one(&mut *tx)
            .await?,
        0
    );
    tx.commit().await?;
    db.finish().await
}

/// Leaving the service commits while sign-in is verifying the password it has already read.
/// The two transactions genuinely interleave: sign-in's verifier blocks until the departure
/// has committed, then returns success. Without the actor lock and the second read of
/// `users`, the session insert failed on the deleted row as `Database` — a 500 where the
/// contract says 401.
#[tokio::test]
async fn a_departure_committed_during_verification_is_invalid_credentials() -> Result {
    let db = Database::new().await?;
    let email = "racing@example.invalid";
    let mut tx = db.app.begin().await?;
    let actor = db::create_person(&mut tx, email, support::PASSWORD_HASH, "Fixture person").await?;
    tx.commit().await?;

    let (verifying, verifying_seen) = std::sync::mpsc::channel::<()>();
    let (departed, departed_seen) = std::sync::mpsc::channel::<()>();
    let departed_seen = std::sync::Mutex::new(departed_seen);
    let pool = db.app.clone();
    let sign_in = tokio::spawn(async move {
        db::sign_in(
            &pool,
            email,
            move |_| {
                let _ = verifying.send(());
                let _ = departed_seen.lock().map(|receiver| receiver.recv());
                true
            },
            &hash('c'),
            OffsetDateTime::now_utc() + Duration::days(14),
        )
        .await
    });
    tokio::task::spawn_blocking(move || verifying_seen.recv()).await??;
    db::leave_service(&db.app, actor).await?;
    departed.send(())?;

    let outcome = sign_in.await?;
    assert!(
        matches!(outcome, Err(db::SignInError::InvalidCredentials)),
        "{outcome:?}"
    );
    assert!(matches!(
        db::resolve_session(&db.app, &hash('c')).await,
        Err(db::SessionError::NoSession)
    ));
    db.finish().await
}

#[tokio::test]
async fn sessions_resolve_revoke_only_the_named_session_and_leave_with_the_person() -> Result {
    let db = Database::new().await?;
    let mut tx = db.app.begin().await?;
    let actor = db::create_person(
        &mut tx,
        "ASCII@example.invalid",
        support::PASSWORD_HASH,
        "Fixture person",
    )
    .await?;
    tx.commit().await?;
    let expiry = OffsetDateTime::now_utc() + Duration::days(14);
    for token in [hash('a'), hash('b')] {
        assert_eq!(
            db::sign_in(&db.app, "ascii@EXAMPLE.invalid", |_| true, &token, expiry).await?,
            actor
        );
        assert_eq!(db::resolve_session(&db.app, &token).await?, actor);
    }
    let other = db.person().await?;
    assert!(matches!(
        db::revoke_session(&db.app, other, &hash('a')).await,
        Err(db::SessionError::NoSession)
    ));
    for _ in 0..2 {
        db::revoke_session(&db.app, actor, &hash('a')).await?;
    }
    assert!(matches!(
        db::resolve_session(&db.app, &hash('a')).await,
        Err(db::SessionError::NoSession)
    ));
    assert_eq!(db::resolve_session(&db.app, &hash('b')).await?, actor);
    assert!(matches!(
        db::resolve_session(&db.app, &hash('c')).await,
        Err(db::SessionError::NoSession)
    ));
    assert!(matches!(
        db::revoke_session(&db.app, actor, &hash('c')).await,
        Err(db::SessionError::NoSession)
    ));
    db::leave_service(&db.app, actor).await?;
    assert!(matches!(
        db::me(&db.app, actor).await,
        Err(db::SessionError::NoSession)
    ));
    assert!(matches!(
        db::resolve_session(&db.app, &hash('b')).await,
        Err(db::SessionError::NoSession)
    ));
    assert!(matches!(
        db::sign_in(
            &db.app,
            "ASCII@example.invalid",
            |_| true,
            &hash('c'),
            expiry
        )
        .await,
        Err(db::SignInError::InvalidCredentials)
    ));
    let mut tx = db.actor(actor).await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions WHERE actor_id = $1")
            .bind(actor.0)
            .fetch_one(&mut *tx)
            .await?,
        0
    );
    tx.commit().await?;
    db.finish().await
}

#[tokio::test]
async fn nonhuman_me_has_no_email_and_cannot_hold_a_session() -> Result {
    let db = Database::new().await?;
    let actor = ActorId(Uuid::now_v7());
    let mut tx = db.actor(actor).await?;
    sqlx::query("INSERT INTO actors (id, type, display_name) VALUES ($1, 'bot', 'Fixture bot')")
        .bind(actor.0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let me = db::me(&db.app, actor).await?;
    assert_eq!(me.actor_type, ActorType::Bot);
    assert_eq!(me.display_name.as_deref(), Some("Fixture bot"));
    assert_eq!(me.email, None);
    let mut tx = db.actor(actor).await?;
    support::rejected(sqlx::query("INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, $2, $3, now() + interval '14 days')").bind(Uuid::now_v7()).bind(actor.0).bind(hash('a').as_str()).execute(&mut *tx).await, "23503");
    tx.rollback().await?;
    db.finish().await
}

#[tokio::test]
async fn absolute_expiry_is_not_extended_by_resolution() -> Result {
    let db = Database::new().await?;
    support::sessions::clock_at_sign_in(&db).await?;
    let mut tx = db.app.begin().await?;
    let actor = db::create_person(
        &mut tx,
        "clock@example.invalid",
        support::PASSWORD_HASH,
        "Fixture person",
    )
    .await?;
    tx.commit().await?;
    let expiry = support::sessions::fixed_now() + Duration::days(14);
    db::sign_in(
        &db.app,
        "clock@example.invalid",
        |_| true,
        &hash('a'),
        expiry,
    )
    .await?;
    let mut attempt = db.actor(actor).await?;
    support::rejected(sqlx::query("UPDATE sessions SET expires_at = expires_at + interval '1 day' WHERE actor_id = $1 AND token_hash = $2").bind(actor.0).bind(hash('a').as_str()).execute(&mut *attempt).await, "42501");
    attempt.rollback().await?;
    support::sessions::clock_before_expiry(&db).await?;
    assert_eq!(db::resolve_session(&db.app, &hash('a')).await?, actor);
    let row = support::sessions::stored_session(&db, actor, &hash('a')).await?;
    assert_eq!(row["expires_at"], "2040-01-15T00:00:00+00:00");
    support::sessions::clock_at_expiry(&db).await?;
    assert!(matches!(
        db::resolve_session(&db.app, &hash('a')).await,
        Err(db::SessionError::NoSession)
    ));
    support::sessions::clock_after_expiry(&db).await?;
    assert!(matches!(
        db::resolve_session(&db.app, &hash('a')).await,
        Err(db::SessionError::NoSession)
    ));
    db.finish().await
}

async fn visible_fixture(db: &Database) -> Result<support::Conversation> {
    let c = support::Conversation::new(db).await?;
    c.post(db).await?;
    db::add_team_member(
        &db.app,
        c.owner,
        c.organization,
        c.team,
        c.owner,
        domain::TeamRole::Member,
    )
    .await?;
    db::add_channel_member(&db.app, c.owner, c.organization, c.channel, c.owner).await?;
    let mut tx = db.actor(c.owner).await?;
    sqlx::query("INSERT INTO invites (id, organization_id, kind, token_hash, expires_at, max_uses, created_by_actor_id) VALUES ($1, $2, 'organization', $3, now() + interval '1 day', 1, $4)").bind(Uuid::now_v7()).bind(c.organization.0).bind(hash('f').as_str()).bind(c.owner.0).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(c)
}

async fn signed_in_person(db: &Database) -> Result<ActorId> {
    let mut tx = db.app.begin().await?;
    let actor = db::create_person(
        &mut tx,
        "visibility@example.invalid",
        db::DUMMY_PASSWORD_HASH,
        "Fixture person",
    )
    .await?;
    tx.commit().await?;
    let actor_from_session = db::sign_in(
        &db.app,
        "visibility@example.invalid",
        |phc| phc == db::DUMMY_PASSWORD_HASH,
        &hash('a'),
        OffsetDateTime::now_utc() + Duration::days(14),
    )
    .await?;
    assert_eq!(actor_from_session, actor);
    Ok(actor)
}

async fn assert_organization_hidden(
    db: &Database,
    c: &support::Conversation,
    actor: ActorId,
    own_memberships: bool,
) -> Result {
    assert_eq!(db::resolve_session(&db.app, &hash('a')).await?, actor);
    let mut tx = db.actor(actor).await?;
    for query in [
        "SELECT count(*) FROM organizations WHERE id = $1",
        "SELECT count(*) FROM teams WHERE organization_id = $1",
        "SELECT count(*) FROM channels WHERE organization_id = $1",
        "SELECT count(*) FROM messages WHERE organization_id = $1",
        "SELECT count(*) FROM invites WHERE organization_id = $1",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(query)
                .bind(c.organization.0)
                .fetch_one(&mut *tx)
                .await?,
            0
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM actors")
            .fetch_all(&mut *tx)
            .await?,
        [actor.0]
    );
    for query in [
        "SELECT actor_id FROM organization_members WHERE organization_id = $1",
        "SELECT actor_id FROM team_members WHERE organization_id = $1",
        "SELECT actor_id FROM channel_members WHERE organization_id = $1",
    ] {
        let rows = sqlx::query_scalar::<_, Uuid>(query)
            .bind(c.organization.0)
            .fetch_all(&mut *tx)
            .await?;
        assert_eq!(
            rows,
            if own_memberships {
                vec![actor.0]
            } else {
                vec![]
            }
        );
    }
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn signed_in_suspended_member_sees_only_own_memberships() -> Result {
    let db = Database::new().await?;
    let c = visible_fixture(&db).await?;
    let actor = signed_in_person(&db).await?;
    db.member(c.organization, actor, "member").await?;
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
    db::suspend_member(&db.app, c.owner, c.organization, actor).await?;
    assert_eq!(
        db::sign_in(
            &db.app,
            "visibility@example.invalid",
            |_| true,
            &hash('b'),
            OffsetDateTime::now_utc() + Duration::days(14)
        )
        .await?,
        actor
    );
    assert_eq!(db::resolve_session(&db.app, &hash('b')).await?, actor);
    // Suspension neither revokes a session nor makes its actor an active member.
    assert_organization_hidden(&db, &c, actor, true).await?;
    db.finish().await
}

#[tokio::test]
async fn signed_in_other_organization_member_sees_no_rows_or_memberships() -> Result {
    let db = Database::new().await?;
    let c = visible_fixture(&db).await?;
    let actor = signed_in_person(&db).await?;
    let _other = db.organization(actor).await?;
    assert_organization_hidden(&db, &c, actor, false).await?;
    db.finish().await
}
