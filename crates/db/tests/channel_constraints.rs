mod support;
use domain::{ActorId, ChannelScope, OrganizationId, TeamId};
use support::{Conversation, Database, Result, rejected};
use uuid::Uuid;

async fn insert_channel(
    db: &Database,
    org: OrganizationId,
    team: Option<TeamId>,
    scope: &str,
    name: &str,
) -> std::result::Result<Uuid, sqlx::Error> {
    sqlx::query_scalar("INSERT INTO channels (id, organization_id, team_id, scope, name) VALUES ($1, $2, $3, $4, $5) RETURNING id")
        .bind(Uuid::now_v7()).bind(org.0).bind(team.map(|id| id.0)).bind(scope).bind(name).fetch_one(&db.app).await
}

#[tokio::test]
async fn channel_scope_requires_exactly_its_owner_and_match_simple_skips_null_team() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    rejected(
        insert_channel(&db, c.organization, Some(c.team), "organization", "bad").await,
        "23514",
    );
    rejected(
        insert_channel(&db, c.organization, None, "team", "bad").await,
        "23514",
    );
    rejected(
        insert_channel(&db, c.organization, None, "unknown", "bad").await,
        "23514",
    );
    let id = insert_channel(&db, c.organization, None, "organization", "general").await?;
    let counter: i64 = sqlx::query_scalar(
        "SELECT next_message_seq FROM channels WHERE organization_id = $1 AND id = $2",
    )
    .bind(c.organization.0)
    .bind(id)
    .fetch_one(&db.app)
    .await?;
    assert_eq!(counter, 0);
    let match_type: String = sqlx::query_scalar("SELECT confmatchtype::text FROM pg_constraint WHERE conrelid = 'channels'::regclass AND confrelid = 'teams'::regclass").fetch_one(&db.owner).await?;
    assert_eq!(match_type, "s");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channels WHERE organization_id = $1")
        .bind(c.organization.0)
        .fetch_one(&db.app)
        .await?;
    assert_eq!(count, 2);
    db.finish().await
}

#[tokio::test]
async fn channel_team_must_belong_to_its_organization() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    rejected(
        insert_channel(&db, a.organization, Some(b.team), "team", "bad").await,
        "23503",
    );
    rejected(
        insert_channel(
            &db,
            OrganizationId(Uuid::now_v7()),
            None,
            "organization",
            "bad",
        )
        .await,
        "23503",
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channels WHERE organization_id = $1")
        .bind(a.organization.0)
        .fetch_one(&db.app)
        .await?;
    assert_eq!(count, 1);
    db.finish().await
}

#[tokio::test]
async fn channel_names_are_unique_per_owner_and_ascii_case_insensitive() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let other = db::create_team(&db.app, c.owner, c.organization, "Other team").await?;
    insert_channel(&db, c.organization, Some(other), "team", "general").await?;
    for name in ["general", "GENERAL"] {
        rejected(
            insert_channel(&db, c.organization, Some(c.team), "team", name).await,
            "23505",
        );
    }
    insert_channel(&db, c.organization, None, "organization", "general").await?;
    for name in ["general", "GENERAL"] {
        rejected(
            insert_channel(&db, c.organization, None, "organization", name).await,
            "23505",
        );
    }
    let other_org = db.organization(c.owner).await?;
    insert_channel(&db, other_org, None, "organization", "GENERAL").await?;
    let indexes: Vec<String> = sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE tablename = 'channels' AND indexname IN ('channels_team_name_key', 'channels_organization_name_key')").fetch_all(&db.owner).await?;
    assert_eq!(indexes.len(), 2);
    assert!(
        indexes
            .iter()
            .all(|definition| definition.contains("COLLATE \"C\""))
    );
    db.finish().await
}

#[tokio::test]
async fn channel_name_length_counts_characters_and_refuses_both_boundaries() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    for name in [String::new(), "x".repeat(81)] {
        rejected(
            insert_channel(&db, c.organization, None, "organization", &name).await,
            "23514",
        );
    }
    insert_channel(&db, c.organization, None, "organization", &"界".repeat(80)).await?;
    db.finish().await
}

#[tokio::test]
async fn implicit_channels_cannot_list_members_even_with_a_forged_or_null_scope() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let id = db::create_channel(
        &db.app,
        c.owner,
        c.organization,
        ChannelScope::Organization,
        "general",
    )
    .await?;
    for (scope, code) in [
        (Some("team"), "23503"),
        (Some("organization"), "23514"),
        (None, "23502"),
    ] {
        rejected(sqlx::query("INSERT INTO channel_members (channel_id, channel_scope, actor_id) VALUES ($1, $2, $3)").bind(id.0).bind(scope).bind(c.owner.0).execute(&db.app).await, code);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1").bind(c.organization.0).fetch_one(&db.app).await?;
    assert_eq!(count, 0);
    db.finish().await
}

async fn insert_message(
    db: &Database,
    org: OrganizationId,
    channel: Uuid,
    sender: Option<ActorId>,
    seq: i64,
    body: &str,
) -> std::result::Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO messages (id, organization_id, channel_id, sender_actor_id, channel_seq, body) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(Uuid::now_v7()).bind(org.0).bind(channel).bind(sender.map(|id| id.0)).bind(seq).bind(body).execute(&db.app).await?;
    Ok(())
}

#[tokio::test]
async fn message_foreign_keys_and_sender_nullability_refuse_invalid_rows() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    rejected(
        insert_message(&db, a.organization, b.channel.0, Some(a.owner), 1, "body").await,
        "23503",
    );
    rejected(
        insert_message(&db, a.organization, a.channel.0, None, 1, "body").await,
        "23502",
    );
    let missing = OrganizationId(Uuid::now_v7());
    let error = insert_message(&db, missing, a.channel.0, Some(a.owner), 1, "body")
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().and_then(|e| e.constraint()),
        Some("messages_organization_id_fkey")
    );
    rejected(Err::<(), _>(error), "23503");
    rejected(
        insert_message(
            &db,
            a.organization,
            a.channel.0,
            Some(ActorId(Uuid::now_v7())),
            1,
            "body",
        )
        .await,
        "23503",
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM messages WHERE organization_id = $1 OR organization_id = $2 OR organization_id = $3").bind(a.organization.0).bind(b.organization.0).bind(missing.0).fetch_one(&db.app).await?;
    assert_eq!(count, 0);
    db.finish().await
}

#[tokio::test]
async fn message_body_and_sequence_constraints_are_backstops() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    for body in [String::new(), "x".repeat(4001)] {
        rejected(
            insert_message(&db, c.organization, c.channel.0, Some(c.owner), 1, &body).await,
            "23514",
        );
    }
    rejected(
        insert_message(&db, c.organization, c.channel.0, Some(c.owner), 0, "body").await,
        "23514",
    );
    insert_message(
        &db,
        c.organization,
        c.channel.0,
        Some(c.owner),
        1,
        &"界".repeat(4000),
    )
    .await?;
    rejected(
        insert_message(
            &db,
            c.organization,
            c.channel.0,
            Some(c.owner),
            1,
            "duplicate",
        )
        .await,
        "23505",
    );
    rejected(
        sqlx::query(
            "UPDATE channels SET next_message_seq = -1 WHERE organization_id = $1 AND id = $2",
        )
        .bind(c.organization.0)
        .bind(c.channel.0)
        .execute(&db.app)
        .await,
        "23514",
    );
    db.finish().await
}
