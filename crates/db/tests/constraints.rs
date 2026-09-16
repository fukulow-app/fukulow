mod support;
use support::{Database, PASSWORD_HASH, Result, rejected};
use uuid::Uuid;

struct Fixture {
    db: Database,
    human: Uuid,
    bot: Uuid,
    org: Uuid,
    other_org: Uuid,
    team: Uuid,
    other_team: Uuid,
}
impl Fixture {
    async fn new() -> Result<Self> {
        let db = Database::new().await?;
        let human = db.person().await?;
        let bot = Uuid::now_v7();
        sqlx::query("INSERT INTO actors (id, type) VALUES ($1, 'bot')")
            .bind(bot)
            .execute(&db.app)
            .await?;
        let org = db.organization(human).await?;
        let other_org = db.organization(human).await?;
        let team = db::create_team(&db.app, human, org, "Fixture team").await?;
        let other_team = db::create_team(&db.app, human, other_org, "Other fixture team").await?;
        Ok(Self {
            db,
            human: human.0,
            bot,
            org: org.0,
            other_org: other_org.0,
            team: team.0,
            other_team: other_team.0,
        })
    }
}

#[tokio::test]
async fn team_member_requires_organization_membership() -> Result {
    let f = Fixture::new().await?;
    rejected(
        sqlx::query(
            "INSERT INTO team_members (team_id, actor_id, organization_id) VALUES ($1, $2, $3)",
        )
        .bind(f.team)
        .bind(f.bot)
        .bind(f.org)
        .execute(&f.db.app)
        .await,
        "23503",
    );
    f.db.finish().await
}

#[tokio::test]
async fn actor_requires_explicit_type() -> Result {
    let f = Fixture::new().await?;
    rejected(
        sqlx::query("INSERT INTO actors (id) VALUES ($1)")
            .bind(Uuid::now_v7())
            .execute(&f.db.app)
            .await,
        "23502",
    );
    f.db.finish().await
}

#[tokio::test]
async fn session_requires_a_person() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, $2, $3, now())").bind(Uuid::now_v7()).bind(f.bot).bind(Uuid::now_v7().to_string()).execute(&f.db.app).await, "23503");
    f.db.finish().await
}

#[tokio::test]
async fn bot_cannot_have_user_row() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO users (actor_id, actor_type, email, password_hash) VALUES ($1, 'human', 'bot@example.invalid', $2)").bind(f.bot).bind(PASSWORD_HASH).execute(&f.db.app).await, "23503");
    f.db.finish().await
}

#[tokio::test]
async fn null_actor_type_cannot_bypass_composite_key() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO users (actor_id, actor_type, email, password_hash) VALUES ($1, NULL, 'bot@example.invalid', $2)").bind(f.bot).bind(PASSWORD_HASH).execute(&f.db.app).await, "23502");
    f.db.finish().await
}

#[tokio::test]
async fn null_session_actor_is_refused() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, NULL, $2, now())").bind(Uuid::now_v7()).bind(Uuid::now_v7().to_string()).execute(&f.db.app).await, "23502");
    f.db.finish().await
}

#[tokio::test]
async fn team_member_cannot_cross_organizations() -> Result {
    let f = Fixture::new().await?;
    rejected(
        sqlx::query(
            "INSERT INTO team_members (team_id, actor_id, organization_id) VALUES ($1, $2, $3)",
        )
        .bind(f.team)
        .bind(f.human)
        .bind(f.other_org)
        .execute(&f.db.app)
        .await,
        "23503",
    );
    f.db.finish().await
}

#[tokio::test]
async fn team_requires_existing_organization() -> Result {
    let f = Fixture::new().await?;
    rejected(
        sqlx::query(
            "INSERT INTO teams (id, organization_id, name) VALUES ($1, $2, 'Fixture team')",
        )
        .bind(Uuid::now_v7())
        .bind(Uuid::now_v7())
        .execute(&f.db.app)
        .await,
        "23503",
    );
    f.db.finish().await
}

#[tokio::test]
async fn organization_slug_is_unique() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO organizations (id, name, slug) SELECT $1, name, slug FROM organizations WHERE id = $2").bind(Uuid::now_v7()).bind(f.org).execute(&f.db.app).await, "23505");
    f.db.finish().await
}

#[tokio::test]
async fn organization_slug_format_is_checked() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, 'Fixture organization', 'Bad_Slug')").bind(Uuid::now_v7()).execute(&f.db.app).await, "23514");
    f.db.finish().await
}

#[tokio::test]
async fn organization_membership_pair_is_unique() -> Result {
    let f = Fixture::new().await?;
    rejected(
        sqlx::query("INSERT INTO organization_members (organization_id, actor_id) VALUES ($1, $2)")
            .bind(f.org)
            .bind(f.human)
            .execute(&f.db.app)
            .await,
        "23505",
    );
    f.db.finish().await
}

#[tokio::test]
async fn explicit_null_member_role_is_refused() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO organization_members (organization_id, actor_id, role) VALUES ($1, $2, NULL)").bind(f.org).bind(f.bot).execute(&f.db.app).await, "23502");
    f.db.finish().await
}

#[tokio::test]
async fn organization_member_requires_existing_organization() -> Result {
    let f = Fixture::new().await?;
    rejected(
        sqlx::query("INSERT INTO organization_members (organization_id, actor_id) VALUES ($1, $2)")
            .bind(Uuid::now_v7())
            .bind(f.bot)
            .execute(&f.db.app)
            .await,
        "23503",
    );
    f.db.finish().await
}

#[tokio::test]
async fn organization_member_requires_existing_actor() -> Result {
    let f = Fixture::new().await?;
    rejected(
        sqlx::query("INSERT INTO organization_members (organization_id, actor_id) VALUES ($1, $2)")
            .bind(f.org)
            .bind(Uuid::now_v7())
            .execute(&f.db.app)
            .await,
        "23503",
    );
    f.db.finish().await
}

#[tokio::test]
async fn invite_requires_creator() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, 'organization', NULL, $3, now(), 1, 0, NULL)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).execute(&f.db.app).await, "23502");
    f.db.finish().await
}

#[tokio::test]
async fn invite_requires_existing_creator() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, 'organization', NULL, $3, now(), 1, 0, $5)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).bind(Uuid::now_v7()).execute(&f.db.app).await, "23503");
    f.db.finish().await
}

#[tokio::test]
async fn invite_requires_existing_organization() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $5, 'organization', NULL, $3, now(), 1, 0, $4)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).bind(Uuid::now_v7()).execute(&f.db.app).await, "23503");
    f.db.finish().await
}

#[tokio::test]
async fn invite_requires_kind() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, NULL, NULL, $3, now(), 1, 0, $4)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).execute(&f.db.app).await, "23502");
    f.db.finish().await
}

#[tokio::test]
async fn invite_requires_expiration() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, 'organization', NULL, $3, NULL, 1, 0, $4)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).execute(&f.db.app).await, "23502");
    f.db.finish().await
}

#[tokio::test]
async fn team_invite_requires_target() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, 'team', NULL, $3, now(), 1, 0, $4)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).execute(&f.db.app).await, "23514");
    f.db.finish().await
}

#[tokio::test]
async fn organization_invite_forbids_target() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, 'organization', $5, $3, now(), 1, 0, $4)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).bind(f.team).execute(&f.db.app).await, "23514");
    f.db.finish().await
}

#[tokio::test]
async fn invite_count_cannot_exceed_limit() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, 'organization', NULL, $3, now(), 1, 2, $4)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).execute(&f.db.app).await, "23514");
    f.db.finish().await
}

#[tokio::test]
async fn invite_limit_is_positive() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, 'organization', NULL, $3, now(), 0, 0, $4)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).execute(&f.db.app).await, "23514");
    f.db.finish().await
}

#[tokio::test]
async fn invite_count_is_nonnegative() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, used_count, created_by_actor_id) VALUES ($1, $2, 'organization', NULL, $3, now(), 1, -1, $4)").bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).execute(&f.db.app).await, "23514");
    f.db.finish().await
}

#[tokio::test]
async fn team_invite_cannot_cross_organizations() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO invites (id, organization_id, kind, target_team_id, token_hash, expires_at, max_uses, created_by_actor_id) VALUES ($1, $2, 'team', $3, $4, now(), 1, $5)").bind(Uuid::now_v7()).bind(f.org).bind(f.other_team).bind(Uuid::now_v7().to_string()).bind(f.human).execute(&f.db.app).await, "23503");
    f.db.finish().await
}

#[tokio::test]
async fn audit_requires_actor() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO audit_events (id, organization_id, actor_id, action, target_type, target_id, metadata) VALUES ($1, $2, $3, 'organization.created', 'organization', $2, '{}')").bind(Uuid::now_v7()).bind(f.org).bind(None::<Uuid>).execute(&f.db.app).await, "23502");
    f.db.finish().await
}

#[tokio::test]
async fn audit_requires_existing_organization() -> Result {
    let f = Fixture::new().await?;
    rejected(sqlx::query("INSERT INTO audit_events (id, organization_id, actor_id, action, target_type, target_id, metadata) VALUES ($1, $2, $3, 'organization.created', 'organization', $2, '{}')").bind(Uuid::now_v7()).bind(Uuid::now_v7()).bind(Some(f.human)).execute(&f.db.app).await, "23503");
    f.db.finish().await
}

#[tokio::test]
async fn user_requires_email() -> Result {
    let f = Fixture::new().await?;
    sqlx::query("DELETE FROM users WHERE actor_id = $1")
        .bind(f.human)
        .execute(&f.db.app)
        .await?;
    rejected(sqlx::query("INSERT INTO users (actor_id, actor_type, email, password_hash) VALUES ($1, 'human', NULL, $2)").bind(f.human).bind(PASSWORD_HASH).execute(&f.db.app).await, "23502");
    f.db.finish().await
}

#[tokio::test]
async fn user_requires_password_hash() -> Result {
    let f = Fixture::new().await?;
    sqlx::query("DELETE FROM users WHERE actor_id = $1")
        .bind(f.human)
        .execute(&f.db.app)
        .await?;
    rejected(sqlx::query("INSERT INTO users (actor_id, actor_type, email, password_hash) VALUES ($1, 'human', 'fixture@example.invalid', NULL)").bind(f.human).execute(&f.db.app).await, "23502");
    f.db.finish().await
}

#[tokio::test]
async fn declared_defaults_apply_when_omitted() -> Result {
    let f = Fixture::new().await?;
    let (role, status): (String, String) = sqlx::query_as("INSERT INTO organization_members (organization_id, actor_id) VALUES ($1, $2) RETURNING role, status")
        .bind(f.org).bind(f.bot).fetch_one(&f.db.app).await?;
    assert_eq!((role.as_str(), status.as_str()), ("member", "active"));
    let role: String = sqlx::query_scalar("INSERT INTO team_members (organization_id, team_id, actor_id) VALUES ($1, $2, $3) RETURNING role")
        .bind(f.org).bind(f.team).bind(f.bot).fetch_one(&f.db.app).await?;
    assert_eq!(role, "member");
    let used: i32 = sqlx::query_scalar("INSERT INTO invites (id, organization_id, kind, token_hash, expires_at, max_uses, created_by_actor_id) VALUES ($1, $2, 'organization', $3, now(), 1, $4) RETURNING used_count")
        .bind(Uuid::now_v7()).bind(f.org).bind(Uuid::now_v7().to_string()).bind(f.human).fetch_one(&f.db.app).await?;
    assert_eq!(used, 0);
    f.db.finish().await
}

#[tokio::test]
async fn ascii_email_case_is_unique_and_c_collation_is_explicit() -> Result {
    let f = Fixture::new().await?;
    for email in ["I@example.invalid", "Fixture@Example.invalid"] {
        sqlx::query("UPDATE actors SET display_name = NULL WHERE id = $1")
            .bind(f.human)
            .execute(&f.db.app)
            .await?;
        sqlx::query("DELETE FROM users WHERE actor_id = $1")
            .bind(f.human)
            .execute(&f.db.app)
            .await?;
        sqlx::query("INSERT INTO users (actor_id, actor_type, email, password_hash) VALUES ($1, 'human', $2, $3)")
            .bind(f.human).bind(email).bind(PASSWORD_HASH).execute(&f.db.app).await?;
        let second = Uuid::now_v7();
        sqlx::query("INSERT INTO actors (id, type) VALUES ($1, 'human')")
            .bind(second)
            .execute(&f.db.app)
            .await?;
        rejected(sqlx::query("INSERT INTO users (actor_id, actor_type, email, password_hash) VALUES ($1, 'human', $2, $3)")
            .bind(second).bind(email.to_ascii_lowercase()).bind(PASSWORD_HASH).execute(&f.db.app).await, "23505");
    }
    let expression: String = sqlx::query_scalar("SELECT pg_get_indexdef(indexrelid) FROM pg_index WHERE indexrelid = 'users_email_lower_key'::regclass")
        .fetch_one(&f.db.owner).await?;
    assert!(expression.contains("lower((email COLLATE \"C\"))"));
    f.db.finish().await
}

#[tokio::test]
async fn invite_and_session_hashes_are_unique() -> Result {
    let f = Fixture::new().await?;
    let hash = Uuid::now_v7().to_string();
    for second in [false, true] {
        let invite = sqlx::query("INSERT INTO invites (id, organization_id, kind, token_hash, expires_at, max_uses, created_by_actor_id) VALUES ($1, $2, 'organization', $3, now(), 1, $4)")
            .bind(Uuid::now_v7()).bind(f.org).bind(&hash).bind(f.human).execute(&f.db.app).await;
        let session = sqlx::query("INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, $2, $3, now())")
            .bind(Uuid::now_v7()).bind(f.human).bind(&hash).execute(&f.db.app).await;
        if second {
            rejected(invite, "23505");
            rejected(session, "23505");
        } else {
            invite?;
            session?;
        }
    }
    f.db.finish().await
}
