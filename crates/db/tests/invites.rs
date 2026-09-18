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
use domain::{ActorId, InviteId, OrganizationId};
use sqlx::{PgPool, postgres::PgPoolOptions};
use support::{Database, Result, invites as fixtures, rejected};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

async fn fixture(
    db: &Database,
    max_uses: i32,
) -> Result<(ActorId, OrganizationId, InviteId, db::TokenHash)> {
    let actor = db.person().await?;
    let org = db.organization(actor).await?;
    let hash = fixture_hash();
    let id = fixtures::invite(db, actor, org, &hash, max_uses).await?;
    Ok((actor, org, id, hash))
}

fn fixture_hash() -> db::TokenHash {
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(Uuid::now_v7().as_bytes());
    db::TokenHash::from_bytes(bytes)
}

async fn presented<'a>(
    pool: &'a PgPool,
    hash: &db::TokenHash,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('fukulow.invite_token_hash', $1, true)")
        .bind(hash.as_str())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

#[tokio::test]
async fn acceptance_rechecks_every_unusable_kind_and_rolls_back_its_conditional_use() -> Result {
    let db = Database::new().await?;
    let (actor, org, _, _) = fixture(&db, 1).await?;
    for kind in ["unknown", "expired", "revoked", "exhausted", "team"] {
        let hash = fixture_hash();
        let id = fixtures::unusable(&db, actor, org, &hash, kind).await?;
        let before = fixtures::counts(&db).await?;
        assert!(!db::invite_is_usable(&db.app, &hash).await?);
        let stored = match id {
            Some(id) => Some(fixtures::stored(&db, org, id).await?),
            None => None,
        };
        assert!(matches!(
            db::accept_invite(
                &db.app,
                &hash,
                "unused@example.invalid",
                support::PASSWORD_HASH,
                "Fixture joiner"
            )
            .await,
            Err(db::AcceptInviteError::InviteUnusable)
        ));
        assert_eq!(fixtures::counts(&db).await?, before);
        if let Some(id) = id {
            assert_eq!(Some(fixtures::stored(&db, org, id).await?), stored);
        }
    }
    db.finish().await
}

#[tokio::test]
async fn membership_requires_a_successful_increment_in_this_transaction() -> Result {
    let db = Database::new().await?;
    let (_, org, id, hash) = fixture(&db, 2).await?;
    // A previous committed use must not authorize another actor, even while capacity remains.
    fixtures::consume(&db.inspector, org, id).await?;
    let before = fixtures::counts(&db).await?;
    let mut tx = presented(&db.app, &hash).await?;
    let actor = db::create_person(
        &mut tx,
        "no-increment@example.invalid",
        support::PASSWORD_HASH,
        "Fixture joiner",
    )
    .await?;
    rejected(sqlx::query("INSERT INTO organization_members (organization_id, actor_id, role, status) VALUES ($1, $2, 'member', 'active')")
        .bind(org.0).bind(actor.0).execute(&mut *tx).await, "42501");
    tx.rollback().await?;
    assert_eq!(fixtures::counts(&db).await?, before);
    db.finish().await
}

#[tokio::test]
async fn exhausted_noop_cannot_manufacture_the_membership_proof() -> Result {
    let db = Database::new().await?;
    let (_, org, id, hash) = fixture(&db, 1).await?;
    fixtures::consume(&db.inspector, org, id).await?;
    let before = fixtures::counts(&db).await?;
    let mut tx = presented(&db.app, &hash).await?;
    rejected(sqlx::query("UPDATE invites SET used_count = used_count WHERE organization_id = $1 AND token_hash = $2")
        .bind(org.0).bind(hash.as_str()).execute(&mut *tx).await, "42501");
    // The failed update aborts the whole transaction, including any attempted insertion.
    rejected(sqlx::query("INSERT INTO organization_members (organization_id, actor_id, role, status) VALUES ($1, $2, 'member', 'active')")
        .bind(org.0).bind(Uuid::now_v7()).execute(&mut *tx).await, "25P02");
    tx.rollback().await?;
    assert_eq!(fixtures::counts(&db).await?, before);
    db.finish().await
}

#[tokio::test]
async fn increment_proof_admits_only_the_acting_member_in_the_invites_organization() -> Result {
    let db = Database::new().await?;
    let (_, org, id, hash) = fixture(&db, 100).await?;
    let other_owner = db.person().await?;
    let other_org = db.organization(other_owner).await?;
    for (role, status, target, acting) in [
        ("owner", "active", org, true),
        ("admin", "active", org, true),
        ("member", "suspended", org, true),
        ("member", "left", org, true),
        ("member", "active", other_org, true),
        ("member", "active", org, false),
    ] {
        let before = fixtures::counts(&db).await?;
        let mut tx = presented(&db.app, &hash).await?;
        sqlx::query("UPDATE invites SET used_count = used_count + 1 WHERE organization_id = $1 AND id = $2 AND used_count < max_uses AND revoked_at IS NULL AND expires_at > now()")
            .bind(org.0).bind(id.0).execute(&mut *tx).await?;
        let actor = db::create_person(
            &mut tx,
            "invalid-member@example.invalid",
            support::PASSWORD_HASH,
            "Fixture joiner",
        )
        .await?;
        rejected(sqlx::query("INSERT INTO organization_members (organization_id, actor_id, role, status) VALUES ($1, $2, $3, $4)")
            .bind(target.0).bind(if acting {actor.0} else {other_owner.0}).bind(role).bind(status).execute(&mut *tx).await, "42501");
        tx.rollback().await?;
        assert_eq!(fixtures::counts(&db).await?, before);
    }
    db.finish().await
}

#[tokio::test]
async fn invite_trigger_refuses_fabricated_initial_counts_unbounded_changes_and_unrevocation()
-> Result {
    let db = Database::new().await?;
    let (owner, org, id, hash) = fixture(&db, 2).await?;
    let mut tx = db.actor(owner).await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, 'organization', $3, now() + interval '1 hour', 2, 1, $4)")
        .bind(Uuid::now_v7()).bind(org.0).bind(db::TokenHash::from_bytes([81;32]).as_str()).bind(owner.0).execute(&mut *tx).await, "23514");
    tx.rollback().await?;
    for count in [0, 2, -1] {
        let mut tx = presented(&db.app, &hash).await?;
        rejected(
            sqlx::query(
                "UPDATE invites SET used_count = $3 WHERE organization_id = $1 AND id = $2",
            )
            .bind(org.0)
            .bind(id.0)
            .bind(count)
            .execute(&mut *tx)
            .await,
            "42501",
        );
        tx.rollback().await?;
    }
    // A use that also revokes is refused: a use changes the count and nothing else.
    let mut tx = presented(&db.app, &hash).await?;
    rejected(
        sqlx::query("UPDATE invites SET used_count = used_count + 1, revoked_at = now() WHERE organization_id = $1 AND id = $2")
            .bind(org.0)
            .bind(id.0)
            .execute(&mut *tx)
            .await,
        "42501",
    );
    tx.rollback().await?;
    db::revoke_invite(&db.app, owner, org, Some(id)).await?;
    // After the first revocation, neither un-revoking nor rewriting its time is allowed.
    for statement in [
        "UPDATE invites SET revoked_at = NULL WHERE organization_id = $1 AND id = $2",
        "UPDATE invites SET revoked_at = now() + interval '1 day' WHERE organization_id = $1 AND id = $2",
    ] {
        let mut tx = presented(&db.app, &hash).await?;
        rejected(
            sqlx::query(statement)
                .bind(org.0)
                .bind(id.0)
                .execute(&mut *tx)
                .await,
            "42501",
        );
        tx.rollback().await?;
    }
    db.finish().await
}

#[tokio::test]
async fn precheck_sees_only_its_invite_and_acceptance_context_does_not_leak() -> Result {
    let db = Database::new().await?;
    let (_, org, _, hash) = fixture(&db, 2).await?;
    let _ = fixture(&db, 1).await?;
    populate_other_tables(&db).await?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.app.connect_options().as_ref().clone())
        .await?;
    let mut tx = presented(&pool, &hash).await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT token_hash FROM invites")
            .fetch_all(&mut *tx)
            .await?,
        [hash.as_str()]
    );
    assert_eq!(visible_rows(&mut tx).await?, 1);
    tx.commit().await?;
    assert!(db::invite_is_usable(&pool, &hash).await?);
    let (actor, actual_org) = db::accept_invite(
        &pool,
        &hash,
        "accepted@example.invalid",
        support::PASSWORD_HASH,
        "Fixture joiner",
    )
    .await?;
    assert_eq!(actual_org, org);
    let mut tx = pool.begin().await?;
    assert_eq!(visible_rows(&mut tx).await?, 0);
    tx.commit().await?;
    let joined = fixtures::joined(&db, org, "accepted@example.invalid").await?;
    assert_eq!(joined["actor"]["id"], actor.0.to_string());
    pool.close().await;
    db.finish().await
}

async fn populate_other_tables(db: &Database) -> Result {
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
    sqlx::query("INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, $2, $3, now() + interval '1 hour')")
        .bind(Uuid::now_v7()).bind(c.owner.0).bind(fixture_hash().as_str()).execute(&db.inspector).await?;
    Ok(())
}

async fn visible_rows(conn: &mut sqlx::PgConnection) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT (SELECT count(*) FROM actors) + (SELECT count(*) FROM users) + (SELECT count(*) FROM sessions) + (SELECT count(*) FROM organizations) + (SELECT count(*) FROM organization_members) + (SELECT count(*) FROM teams) + (SELECT count(*) FROM team_members) + (SELECT count(*) FROM channels) + (SELECT count(*) FROM channel_members) + (SELECT count(*) FROM messages) + (SELECT count(*) FROM invites)").fetch_one(conn).await?)
}

#[tokio::test]
async fn trusted_operations_still_refuse_missing_and_foreign_organizations() -> Result {
    let db = Database::new().await?;
    let (actor, org, id, hash) = fixture(&db, 2).await?;
    let outsider = db.person().await?;
    let other = db.organization(outsider).await?;
    let before = fixtures::counts(&db).await?;
    for target in [other, OrganizationId(Uuid::now_v7())] {
        assert!(matches!(
            db::create_invite(
                &db.app,
                actor,
                target,
                fixtures::token(&hash),
                OffsetDateTime::now_utc() + Duration::hours(1),
                1
            )
            .await,
            Err(db::CreateInviteError::NotAMember)
        ));
        assert!(matches!(
            db::revoke_invite(&db.app, actor, target, Some(id)).await,
            Err(db::RevokeInviteError::NotAMember)
        ));
    }
    assert_eq!(fixtures::counts(&db).await?, before);
    assert_eq!(fixtures::stored(&db, org, id).await?["used_count"], 0);
    db.finish().await
}
