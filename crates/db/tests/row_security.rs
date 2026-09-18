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
use support::{Conversation, Database, Result, rejected};
use uuid::Uuid;

#[tokio::test]
async fn every_application_table_enables_and_forces_a_policy() -> Result {
    let db = Database::new().await?;
    let missing: Vec<String> = sqlx::query_scalar("SELECT c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p') AND c.relname <> '_sqlx_migrations' AND (NOT c.relrowsecurity OR NOT c.relforcerowsecurity OR NOT EXISTS (SELECT 1 FROM pg_policy p WHERE p.polrelid = c.oid)) ORDER BY c.relname").fetch_all(&db.inspector).await?;
    assert!(missing.is_empty(), "unprotected tables: {missing:?}");
    db.finish().await
}

macro_rules! no_identity {
    ($name:ident, $query:literal, $fixture:literal) => {
        #[tokio::test]
        async fn $name() -> Result {
            let db = Database::new().await?;
            sqlx::raw_sql(include_str!("fixtures/populated.sql"))
                .execute(&db.inspector)
                .await?;
            sqlx::raw_sql($fixture).execute(&db.inspector).await?;
            assert!(
                sqlx::query_scalar::<_, i64>($query)
                    .fetch_one(&db.inspector)
                    .await?
                    > 0
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>($query)
                    .fetch_one(&db.app)
                    .await?,
                0
            );
            db.finish().await
        }
    };
}
no_identity!(
    no_identity_actors,
    "SELECT count(*) FROM actors",
    "SELECT 1"
);
no_identity!(no_identity_users, "SELECT count(*) FROM users", "SELECT 1");
no_identity!(
    no_identity_sessions,
    "SELECT count(*) FROM sessions",
    "SELECT 1"
);
no_identity!(
    no_identity_organizations,
    "SELECT count(*) FROM organizations",
    "SELECT 1"
);
no_identity!(
    no_identity_organization_members,
    "SELECT count(*) FROM organization_members",
    "SELECT 1"
);
no_identity!(no_identity_teams, "SELECT count(*) FROM teams", "SELECT 1");
no_identity!(
    no_identity_team_members,
    "SELECT count(*) FROM team_members",
    "SELECT 1"
);
no_identity!(
    no_identity_channels,
    "SELECT count(*) FROM channels",
    "SELECT 1"
);
no_identity!(
    no_identity_channel_members,
    "SELECT count(*) FROM channel_members",
    "SELECT 1"
);
no_identity!(
    no_identity_messages,
    "SELECT count(*) FROM messages",
    "SELECT 1"
);
no_identity!(
    no_identity_invites,
    "SELECT count(*) FROM invites",
    "SELECT 1"
);

#[tokio::test]
async fn audit_and_migrations_are_not_readable() -> Result {
    let db = Database::new().await?;
    for query in [
        "SELECT * FROM audit_events",
        "SELECT * FROM _sqlx_migrations",
    ] {
        rejected(sqlx::query(query).fetch_all(&db.app).await, "42501");
    }
    db.finish().await
}

#[tokio::test]
async fn exactly_two_hardened_definers_exist() -> Result {
    let db = Database::new().await?;
    // `r` (restricted) lets the policy's InitPlan run in the leader while messages are scanned
    // in parallel; `s` (safe) would let a worker run the definer, which is not needed.
    let functions: Vec<(String, String, Vec<String>, String, bool)> = sqlx::query_as("SELECT p.proname::text, r.rolname::text, p.proconfig, p.proparallel::text, EXISTS (SELECT 1 FROM aclexplode(coalesce(p.proacl, acldefault('f',p.proowner))) a WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE') FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace JOIN pg_roles r ON r.oid = p.proowner WHERE n.nspname = 'public' AND p.prosecdef ORDER BY p.proname").fetch_all(&db.inspector).await?;
    assert_eq!(
        functions,
        vec![
            (
                "fukulow_active_organizations".into(),
                "fukulow_migrator".into(),
                vec!["search_path=pg_catalog, pg_temp".into()],
                "r".into(),
                false
            ),
            (
                "fukulow_organization_has_members".into(),
                "fukulow_migrator".into(),
                vec!["search_path=pg_catalog, pg_temp".into()],
                "u".into(),
                false
            ),
        ]
    );
    rejected(
        sqlx::query("SET ROLE fukulow_migrator")
            .execute(&db.app)
            .await,
        "42501",
    );
    db.finish().await
}

#[tokio::test]
async fn existing_organization_cannot_be_claimed_as_first_owner() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let stranger = db.person().await?;
    let mut tx = db.actor(stranger).await?;
    rejected(sqlx::query("INSERT INTO organization_members (organization_id, actor_id, role) VALUES ($1, $2, 'owner')").bind(c.organization.0).bind(stranger.0).execute(&mut *tx).await, "42501");
    tx.rollback().await?;
    let org = db.organization(stranger).await?;
    let mut tx = db.actor(stranger).await?;
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM organizations")
            .fetch_all(&mut *tx)
            .await?,
        [org.0]
    );
    tx.rollback().await?;
    db.finish().await
}

#[tokio::test]
async fn inactive_role_guard_allows_admin_changes_and_departure() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let member = db.person().await?;
    db.member(c.organization, member, "member").await?;
    db::add_team_member(
        &db.app,
        c.owner,
        c.organization,
        c.team,
        member,
        domain::TeamRole::Member,
    )
    .await?;
    db::suspend_member(&db.app, c.owner, c.organization, member).await?;
    db::change_member_role(
        &db.app,
        c.owner,
        c.organization,
        member,
        domain::OrganizationRole::Admin,
    )
    .await?;
    let mut tx = db.actor(member).await?;
    assert_eq!(sqlx::query("UPDATE organization_members SET status = 'left', display_name = NULL WHERE organization_id = $1 AND actor_id = $2").bind(c.organization.0).bind(member.0).execute(&mut *tx).await?.rows_affected(), 1);
    tx.commit().await?;
    let role: String = sqlx::query_scalar(
        "SELECT role FROM organization_members WHERE organization_id = $1 AND actor_id = $2",
    )
    .bind(c.organization.0)
    .bind(member.0)
    .fetch_one(&db.inspector)
    .await?;
    assert_eq!(role, "admin");
    db.finish().await
}

#[tokio::test]
async fn suspended_departure_audits_the_channel_organization_without_seeing_channels() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let member = db.person().await?;
    db.member(c.organization, member, "member").await?;
    db::add_channel_member(&db.app, c.owner, c.organization, c.channel, member).await?;
    db::suspend_member(&db.app, c.owner, c.organization, member).await?;
    let mut tx = db.actor(member).await?;
    let own: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members WHERE actor_id = $1")
        .bind(member.0)
        .fetch_one(&mut *tx)
        .await?;
    let reachable: i64 = sqlx::query_scalar("SELECT count(*) FROM channel_members m JOIN channels c ON c.id = m.channel_id WHERE c.organization_id = $1 AND m.actor_id = $2").bind(c.organization.0).bind(member.0).fetch_one(&mut *tx).await?;
    assert_eq!((own, reachable), (1, 0));
    tx.rollback().await?;
    db::leave_service(&db.app, member).await?;
    let records: Vec<_> = db
        .audit(c.organization)
        .await?
        .into_iter()
        .filter(|record| record.action == "channel.member.removed")
        .collect();
    assert_eq!(
        records,
        [support::Audit {
            actor: member.0,
            action: "channel.member.removed".into(),
            target_type: "actor".into(),
            target: member.0,
            metadata: serde_json::json!({"channel_id": c.channel.0, "reason": "left_service"}),
        }]
    );
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM channel_members WHERE organization_id = $1 AND actor_id = $2",
    )
    .bind(c.organization.0)
    .bind(member.0)
    .fetch_one(&db.inspector)
    .await?;
    assert_eq!(remaining, 0);
    db.finish().await
}

#[tokio::test]
async fn credentials_admit_exactly_the_presented_row() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    let hash = "a".repeat(64);
    for (c, value) in [(&a, hash.clone()), (&b, "b".repeat(64))] {
        sqlx::query("INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, $2, $3, now())").bind(Uuid::now_v7()).bind(c.owner.0).bind(&value).execute(&db.inspector).await?;
        sqlx::query("INSERT INTO invites (id, organization_id, kind, token_hash, expires_at, max_uses, created_by_actor_id) VALUES ($1, $2, 'organization', $3, now(), 1, $4)").bind(Uuid::now_v7()).bind(c.organization.0).bind(&value).bind(c.owner.0).execute(&db.inspector).await?;
    }
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE actor_id = $1")
        .bind(a.owner.0)
        .fetch_one(&db.inspector)
        .await?;
    for (setting, value, query) in [
        (
            "fukulow.session_token_hash",
            hash.clone(),
            "SELECT count(*) FROM sessions",
        ),
        (
            "fukulow.invite_token_hash",
            hash,
            "SELECT count(*) FROM invites",
        ),
        (
            "fukulow.sign_in_email",
            email.to_uppercase(),
            "SELECT count(*) FROM users",
        ),
    ] {
        for (value, expected) in [(value, 1_i64), ("wrong-fixture".into(), 0)] {
            let mut tx = db.app.begin().await?;
            sqlx::query("SELECT set_config($1, $2, true)")
                .bind(setting)
                .bind(value)
                .execute(&mut *tx)
                .await?;
            assert_eq!(
                sqlx::query_scalar::<_, i64>(query)
                    .fetch_one(&mut *tx)
                    .await?,
                expected
            );
            tx.rollback().await?;
        }
    }
    db.finish().await
}

#[tokio::test]
async fn membership_functions_use_real_rows_and_return_only_their_contract() -> Result {
    let db = Database::new().await?;
    let a = Conversation::new(&db).await?;
    let b = Conversation::new(&db).await?;
    let c = Conversation::new(&db).await?;
    db.member(b.organization, a.owner, "member").await?;
    db::suspend_member(&db.app, b.owner, b.organization, a.owner).await?;
    db.member(c.organization, a.owner, "member").await?;
    sqlx::query("UPDATE organization_members SET status = 'left' WHERE organization_id = $1 AND actor_id = $2").bind(c.organization.0).bind(a.owner.0).execute(&db.inspector).await?;
    let empty = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO organizations (id, name, slug) VALUES ($1, 'Fixture empty', 'fixture-empty')",
    )
    .bind(empty)
    .execute(&db.inspector)
    .await?;
    let none: Vec<Uuid> = sqlx::query_scalar("SELECT fukulow_active_organizations()")
        .fetch_all(&db.app)
        .await?;
    assert!(none.is_empty());
    let mut tx = db.actor(a.owner).await?;
    for attack in [false, true] {
        if attack {
            sqlx::raw_sql("CREATE TEMP TABLE organization_members (organization_id uuid, actor_id uuid, status text); GRANT SELECT ON organization_members TO PUBLIC").execute(&mut *tx).await?;
            sqlx::query("INSERT INTO pg_temp.organization_members VALUES ($1, $2, 'active')")
                .bind(empty)
                .bind(a.owner.0)
                .execute(&mut *tx)
                .await?;
        }
        let active: Vec<Uuid> = sqlx::query_scalar("SELECT public.fukulow_active_organizations()")
            .fetch_all(&mut *tx)
            .await?;
        assert_eq!(active, [a.organization.0]);
        for (org, expected) in [
            (b.organization.0, true),
            (empty, false),
            (Uuid::now_v7(), false),
        ] {
            let result: Vec<bool> =
                sqlx::query_scalar("SELECT public.fukulow_organization_has_members($1)")
                    .bind(org)
                    .fetch_all(&mut *tx)
                    .await?;
            assert_eq!(result, [expected]);
        }
    }
    tx.rollback().await?;
    db.finish().await
}

#[tokio::test]
async fn co_member_actor_locks_work_and_unrelated_actors_are_not_found() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let member = db.person().await?;
    let stranger = db.person().await?;
    db.member(c.organization, member, "member").await?;
    db::add_team_member(
        &db.app,
        c.owner,
        c.organization,
        c.team,
        member,
        domain::TeamRole::Member,
    )
    .await?;
    // FOR SHARE alone did not reject a narrowed UPDATE policy on the test server.
    let mut tx = db.actor(c.owner).await?;
    assert_eq!(
        sqlx::query("UPDATE actors SET display_name = display_name WHERE id = $1")
            .bind(member.0)
            .execute(&mut *tx)
            .await?
            .rows_affected(),
        1
    );
    tx.rollback().await?;
    assert!(matches!(
        db::add_team_member(
            &db.app,
            c.owner,
            c.organization,
            c.team,
            stranger,
            domain::TeamRole::Member
        )
        .await,
        Err(db::AddTeamMemberError::NotFound)
    ));
    db.finish().await
}

#[tokio::test]
async fn failed_first_owner_insert_leaves_no_organization() -> Result {
    let db = Database::new().await?;
    let actor = db.person().await?;
    sqlx::query("ALTER TABLE organization_members ADD CONSTRAINT reject_owner_fixture CHECK (role <> 'owner')").execute(&db.owner).await?;
    assert!(
        db::create_organization(&db.app, actor, "Fixture organization", "rejected-owner")
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM organizations")
            .fetch_one(&db.inspector)
            .await?,
        0
    );
    db.finish().await
}

#[tokio::test]
async fn self_referencing_membership_policy_recurses() -> Result {
    let db = Database::new().await?;
    sqlx::raw_sql("CREATE TABLE recursion_organizations (id uuid PRIMARY KEY); CREATE TABLE recursion_members (organization_id uuid REFERENCES recursion_organizations(id), actor_id uuid); ALTER TABLE recursion_members ENABLE ROW LEVEL SECURITY; GRANT SELECT ON recursion_members TO fukulow_app; CREATE POLICY recursive_members ON recursion_members FOR SELECT TO fukulow_app USING (organization_id IN (SELECT organization_id FROM recursion_members))").execute(&db.owner).await?;
    rejected(
        sqlx::query("SELECT * FROM recursion_members")
            .fetch_all(&db.app)
            .await,
        "42P17",
    );
    db.finish().await
}

macro_rules! inactive_self_guard {
    ($name:ident, $query:literal, $remove_guard:literal, $denied:expr) => {
        #[tokio::test]
        async fn $name() -> Result {
            let db = Database::new().await?;
            let c = Conversation::new(&db).await?;
            let member = db.person().await?;
            db.member(c.organization, member, "member").await?;
            db::add_team_member(
                &db.app,
                c.owner,
                c.organization,
                c.team,
                member,
                domain::TeamRole::Member,
            )
            .await?;
            db::suspend_member(&db.app, c.owner, c.organization, member).await?;
            let mut tx = db.actor(member).await?;
            let result = sqlx::query($query)
                .bind(c.organization.0)
                .bind(member.0)
                .execute(&mut *tx)
                .await;
            if $denied {
                rejected(result, "42501");
            } else {
                assert_eq!(result?.rows_affected(), 0);
            }
            tx.rollback().await?;
            sqlx::raw_sql($remove_guard).execute(&db.owner).await?;
            let mut tx = db.actor(member).await?;
            assert_eq!(
                sqlx::query($query)
                    .bind(c.organization.0)
                    .bind(member.0)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected(),
                1
            );
            tx.rollback().await?;
            db.finish().await
        }
    };
}
inactive_self_guard!(
    inactive_self_cannot_reactivate,
    "UPDATE organization_members SET status = 'active' WHERE organization_id = $1 AND actor_id = $2",
    "ALTER POLICY organization_members_update ON organization_members WITH CHECK (true)",
    true
);
inactive_self_guard!(
    inactive_self_cannot_change_role_when_leaving,
    "UPDATE organization_members SET role = 'owner', status = 'left', display_name = NULL WHERE organization_id = $1 AND actor_id = $2",
    "DROP TRIGGER organization_members_guard_inactive_role ON organization_members",
    true
);
inactive_self_guard!(
    inactive_self_cannot_retain_display_name,
    "UPDATE organization_members SET display_name = 'x', status = 'left' WHERE organization_id = $1 AND actor_id = $2",
    "ALTER POLICY organization_members_update ON organization_members WITH CHECK (true)",
    true
);
inactive_self_guard!(
    inactive_self_cannot_raise_team_role,
    "UPDATE team_members SET role = 'manager' WHERE organization_id = $1 AND actor_id = $2",
    "ALTER POLICY team_members_update ON team_members USING (true) WITH CHECK (true)",
    false
);

async fn populate_visibility(db: &Database) -> Result<Conversation> {
    let c = Conversation::new(db).await?;
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
    c.post(db).await?;
    sqlx::query("INSERT INTO invites (id, organization_id, kind, token_hash, expires_at, max_uses, created_by_actor_id) VALUES ($1, $2, 'organization', $3, now(), 1, $4)").bind(Uuid::now_v7()).bind(c.organization.0).bind(Uuid::now_v7().to_string()).bind(c.owner.0).execute(&db.inspector).await?;
    Ok(c)
}

#[tokio::test]
async fn actor_visibility_is_isolated_in_every_readable_tenant_table() -> Result {
    let db = Database::new().await?;
    let a = populate_visibility(&db).await?;
    let b = populate_visibility(&db).await?;
    let mut tx = db.actor(a.owner).await?;
    for query in [
        "SELECT id FROM organizations",
        "SELECT organization_id FROM organization_members",
        "SELECT organization_id FROM teams",
        "SELECT organization_id FROM team_members",
        "SELECT organization_id FROM channels",
        "SELECT organization_id FROM channel_members",
        "SELECT organization_id FROM messages",
        "SELECT organization_id FROM invites",
    ] {
        let stored: Vec<Uuid> = sqlx::query_scalar(query).fetch_all(&db.inspector).await?;
        assert!(stored.contains(&b.organization.0));
        let visible: Vec<Uuid> = sqlx::query_scalar(query).fetch_all(&mut *tx).await?;
        assert_eq!(visible, [a.organization.0], "{query}");
    }
    tx.rollback().await?;
    db.finish().await
}

macro_rules! foreign_insert {
    ($name:ident, $sql:literal, $count:expr, $guard:literal) => {
        #[tokio::test]
        async fn $name() -> Result {
            let db = Database::new().await?;
            let a = Conversation::new(&db).await?;
            let b = Conversation::new(&db).await?;
            let candidate = db.person().await?;
            if $count == 3 {
                db.member(b.organization, candidate, "member").await?;
            }
            let ids = [
                b.organization.0,
                candidate.0,
                b.team.0,
                b.channel.0,
                Uuid::now_v7(),
            ];
            let mut tx = db.actor(a.owner).await?;
            let mut query = sqlx::query($sql);
            for id in &ids[..$count] {
                query = query.bind(id);
            }
            rejected(query.execute(&mut *tx).await, "42501");
            tx.rollback().await?;
            sqlx::query($guard).execute(&db.owner).await?;
            let mut tx = db.actor(a.owner).await?;
            let mut query = sqlx::query($sql);
            for id in &ids[..$count] {
                query = query.bind(id);
            }
            assert_eq!(query.execute(&mut *tx).await?.rows_affected(), 1);
            tx.rollback().await?;
            db.finish().await
        }
    };
}
foreign_insert!(
    foreign_organization_member_insert_is_refused,
    "INSERT INTO organization_members (organization_id, actor_id) VALUES ($1, $2)",
    2,
    "ALTER POLICY organization_members_insert ON organization_members WITH CHECK (true)"
);
foreign_insert!(
    foreign_team_insert_is_refused,
    "INSERT INTO teams (organization_id, id, name) VALUES ($1, $2, 'Foreign fixture')",
    2,
    "ALTER POLICY teams_insert ON teams WITH CHECK (true)"
);
foreign_insert!(
    foreign_team_member_insert_is_refused,
    "INSERT INTO team_members (organization_id, actor_id, team_id) VALUES ($1, $2, $3)",
    3,
    "ALTER POLICY team_members_insert ON team_members WITH CHECK (true)"
);
foreign_insert!(
    foreign_channel_insert_is_refused,
    "INSERT INTO channels (organization_id, id, team_id, scope, name) VALUES ($1, $2, $3, 'team', 'Foreign fixture')",
    3,
    "ALTER POLICY channels_insert ON channels WITH CHECK (true)"
);
foreign_insert!(
    foreign_channel_member_insert_is_refused,
    "INSERT INTO channel_members (organization_id, actor_id, channel_id, channel_scope) SELECT $1, $2, $4, 'team' WHERE $3::uuid IS NOT NULL",
    4,
    "ALTER POLICY channel_members_insert ON channel_members WITH CHECK (true)"
);
foreign_insert!(
    foreign_message_insert_is_refused,
    "INSERT INTO messages (organization_id, sender_actor_id, channel_id, id, channel_seq, body) SELECT $1, $2, $4, $5, 1, 'Fixture body' WHERE $3::uuid IS NOT NULL",
    5,
    "ALTER POLICY messages_insert ON messages WITH CHECK (true)"
);
foreign_insert!(
    foreign_invite_insert_is_refused,
    "INSERT INTO invites (organization_id, created_by_actor_id, target_team_id, id, kind, token_hash, expires_at, max_uses) VALUES ($1, $2, $3, $4, 'team', 'foreign-fixture', now(), 1)",
    4,
    "ALTER POLICY invites_insert ON invites WITH CHECK (true)"
);
foreign_insert!(
    foreign_audit_insert_is_refused,
    "INSERT INTO audit_events (organization_id, actor_id, target_id, id, action, target_type, metadata) VALUES ($1, $2, $3, $4, 'team.created', 'team', '{}')",
    4,
    "ALTER POLICY audit_events_insert ON audit_events WITH CHECK (true)"
);

#[tokio::test]
async fn outside_channel_listing_grants_no_message_access_and_cannot_be_added_by_app() -> Result {
    let db = Database::new().await?;
    let a = populate_visibility(&db).await?;
    let b = Conversation::new(&db).await?;
    assert!(matches!(
        db::add_channel_member(&db.app, a.owner, a.organization, a.channel, b.owner).await,
        Err(db::AddChannelMemberError::NotFound)
    ));
    sqlx::query("INSERT INTO channel_members (organization_id, channel_id, channel_scope, actor_id) VALUES ($1, $2, 'team', $3)").bind(a.organization.0).bind(a.channel.0).bind(b.owner.0).execute(&db.inspector).await?;
    let mut tx = db.actor(b.owner).await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM messages WHERE organization_id = $1 AND channel_id = $2",
    )
    .bind(a.organization.0)
    .bind(a.channel.0)
    .fetch_one(&mut *tx)
    .await?;
    assert_eq!(count, 0);
    tx.rollback().await?;
    db.finish().await
}

#[tokio::test]
async fn inactive_self_visibility_reveals_no_other_rows() -> Result {
    let db = Database::new().await?;
    let c = populate_visibility(&db).await?;
    let member = db.person().await?;
    db.member(c.organization, member, "member").await?;
    db::add_team_member(
        &db.app,
        c.owner,
        c.organization,
        c.team,
        member,
        domain::TeamRole::Member,
    )
    .await?;
    db::add_channel_member(&db.app, c.owner, c.organization, c.channel, member).await?;
    db::suspend_member(&db.app, c.owner, c.organization, member).await?;
    for actor in [c.owner, member] {
        sqlx::query("INSERT INTO sessions (id, actor_id, token_hash, expires_at) VALUES ($1, $2, $3, now())")
            .bind(Uuid::now_v7()).bind(actor.0).bind(Uuid::now_v7().to_string())
            .execute(&db.inspector).await?;
    }
    let mut tx = db.actor(member).await?;
    for query in [
        "SELECT actor_id FROM organization_members",
        "SELECT actor_id FROM team_members",
        "SELECT actor_id FROM channel_members",
        "SELECT id FROM actors",
        "SELECT actor_id FROM users",
        "SELECT actor_id FROM sessions",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, Uuid>(query)
                .fetch_all(&mut *tx)
                .await?,
            [member.0],
            "{query}"
        );
    }
    for query in [
        "SELECT count(*) FROM organizations",
        "SELECT count(*) FROM teams",
        "SELECT count(*) FROM channels",
        "SELECT count(*) FROM messages",
        "SELECT count(*) FROM invites",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(query)
                .fetch_one(&mut *tx)
                .await?,
            0,
            "{query}"
        );
    }
    tx.rollback().await?;
    db.finish().await
}

#[tokio::test]
async fn ordinary_audit_after_status_transition_is_refused() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let member = db.person().await?;
    db.member(c.organization, member, "member").await?;
    let mut tx = db.actor(member).await?;
    sqlx::query("UPDATE organization_members SET status = 'left', display_name = NULL WHERE organization_id = $1 AND actor_id = $2").bind(c.organization.0).bind(member.0).execute(&mut *tx).await?;
    rejected(sqlx::query("INSERT INTO audit_events (id, organization_id, actor_id, action, target_type, target_id, metadata) VALUES ($1, $2, $3, 'team.created', 'team', $4, '{}')").bind(Uuid::now_v7()).bind(c.organization.0).bind(member.0).bind(c.team.0).execute(&mut *tx).await, "42501");
    tx.rollback().await?;
    db.finish().await
}
