mod support;
use support::{Conversation, Database, Result};
use uuid::Uuid;

#[tokio::test]
#[ignore = "Run alone with --ignored --test-threads=1"]
async fn row_security_cost_and_actor_lookup_plans() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    populate(&db, &c).await?;
    index_plans(&db, &c).await?;
    let mut samples = [Vec::new(), Vec::new()];
    for (enabled, times) in [false, true].into_iter().zip(&mut samples) {
        sqlx::query(if enabled {
            "ALTER TABLE messages ENABLE ROW LEVEL SECURITY"
        } else {
            "ALTER TABLE messages DISABLE ROW LEVEL SECURITY"
        })
        .execute(&db.owner)
        .await?;
        for run in 0..12 {
            let mut tx = db.actor(c.owner).await?;
            let plan: Vec<String> = sqlx::query_scalar("EXPLAIN (ANALYZE, BUFFERS) SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 ORDER BY channel_seq DESC LIMIT $3")
                .bind(c.organization.0).bind(c.channel.0).bind(50_i64).fetch_all(&mut *tx).await?;
            tx.rollback().await?;
            let plan = plan.join("\n");
            assert!(
                plan.contains("messages_channel_id_channel_seq_key"),
                "{plan}"
            );
            assert!(plan.contains("rows=50 loops=1"), "{plan}");
            if run == 11 {
                println!("messages_before RLS={enabled}\n{plan}");
            }
            if run != 0 {
                times.push(execution_ms(&plan)?);
            }
        }
    }
    for times in &mut samples {
        times.sort_by(f64::total_cmp);
    }
    let baseline = samples[0][5];
    let secured = samples[1][5];
    println!(
        "500000 messages: baseline median={baseline:.3} ms, RLS median={secured:.3} ms, ratio={:.3}",
        secured / baseline
    );
    organization_count_plans(&db, &c).await?;
    db.finish().await?;
    assert!(secured - baseline <= 0.5, "RLS adds more than 0.5 ms");
    Ok(())
}

fn execution_ms(plan: &str) -> Result<f64> {
    Ok(plan
        .lines()
        .find_map(|line| line.strip_prefix("Execution Time: "))
        .ok_or("missing execution time")?
        .trim_end_matches(" ms")
        .parse()?)
}

async fn populate(db: &Database, c: &Conversation) -> Result {
    let actors: Vec<_> = (0..20_000).map(|_| Uuid::now_v7()).collect();
    let orgs: Vec<_> = (0..199).map(|_| Uuid::now_v7()).collect();
    sqlx::query("INSERT INTO actors (id, type) SELECT unnest($1::uuid[]), 'human'")
        .bind(&actors)
        .execute(&db.inspector)
        .await?;
    sqlx::query("INSERT INTO organizations (id, name, slug) SELECT id, 'Fixture organization', replace(id::text, '-', '') FROM unnest($1::uuid[]) id").bind(&orgs).execute(&db.inspector).await?;
    sqlx::query("INSERT INTO organization_members (organization_id, actor_id, status) SELECT $1, id, CASE WHEN seq % 3 = 0 THEN 'suspended' ELSE 'active' END FROM unnest($2::uuid[]) WITH ORDINALITY AS rows(id,seq)").bind(c.organization.0).bind(&actors).execute(&db.inspector).await?;
    sqlx::query("INSERT INTO team_members (organization_id, team_id, actor_id) SELECT $1, $2, unnest($3::uuid[])").bind(c.organization.0).bind(c.team.0).bind(&actors).execute(&db.inspector).await?;
    let messages: Vec<_> = (0..500_000).map(|_| Uuid::now_v7()).collect();
    sqlx::query("INSERT INTO messages (id, organization_id, channel_id, channel_seq, sender_actor_id, body) SELECT id, $1, $2, seq, $3, 'Fixture body' FROM unnest($4::uuid[]) WITH ORDINALITY AS rows(id,seq)").bind(c.organization.0).bind(c.channel.0).bind(c.owner.0).bind(messages).execute(&db.inspector).await?;
    sqlx::raw_sql("ANALYZE organization_members; ANALYZE team_members; ANALYZE messages")
        .execute(&db.inspector)
        .await?;
    Ok(())
}

async fn index_plans(db: &Database, c: &Conversation) -> Result {
    for (label, ddl) in [
        (
            "no actor index",
            "DROP INDEX organization_members_actor_id_idx; DROP INDEX team_members_actor_id_idx",
        ),
        (
            "partial organization index",
            "CREATE INDEX organization_members_active_actor_id_idx ON organization_members (actor_id) WHERE status = 'active'",
        ),
        (
            "full actor indexes",
            "DROP INDEX organization_members_active_actor_id_idx; CREATE INDEX organization_members_actor_id_idx ON organization_members (actor_id); CREATE INDEX team_members_actor_id_idx ON team_members (actor_id)",
        ),
    ] {
        sqlx::raw_sql(ddl).execute(&db.owner).await?;
        for query in [
            "EXPLAIN (ANALYZE, BUFFERS) SELECT organization_id FROM organization_members WHERE actor_id = $1 AND status = 'active'",
            "EXPLAIN (ANALYZE, BUFFERS) SELECT organization_id FROM organization_members WHERE actor_id = $1",
            "EXPLAIN (ANALYZE, BUFFERS) SELECT organization_id, team_id FROM team_members WHERE actor_id = $1",
        ] {
            let plan: Vec<String> = sqlx::query_scalar(query)
                .bind(c.owner.0)
                .fetch_all(&db.inspector)
                .await?;
            println!("{label}: {query}\n{}", plan.join("\n"));
        }
    }
    let mut tx = db.actor(c.owner).await?;
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN (ANALYZE, BUFFERS) SELECT * FROM fukulow_active_organizations()",
    )
    .fetch_all(&mut *tx)
    .await?;
    println!("active organizations function\n{}", plan.join("\n"));
    tx.rollback().await?;
    Ok(())
}

async fn organization_count_plans(db: &Database, c: &Conversation) -> Result {
    let mut samples = [Vec::new(), Vec::new(), Vec::new()];
    for ((enabled, parallel), times) in [(false, true), (false, false), (true, true)]
        .into_iter()
        .zip(&mut samples)
    {
        sqlx::query(if enabled {
            "ALTER TABLE messages ENABLE ROW LEVEL SECURITY"
        } else {
            "ALTER TABLE messages DISABLE ROW LEVEL SECURITY"
        })
        .execute(&db.owner)
        .await?;
        for run in 0..12 {
            let mut tx = db.actor(c.owner).await?;
            if !parallel {
                sqlx::query("SET LOCAL max_parallel_workers_per_gather = 0")
                    .execute(&mut *tx)
                    .await?;
            }
            let plan: Vec<String> = sqlx::query_scalar("EXPLAIN (ANALYZE, BUFFERS) SELECT count(*) FROM messages WHERE organization_id = $1")
                .bind(c.organization.0).fetch_all(&mut *tx).await?;
            tx.rollback().await?;
            let plan = plan.join("\n");
            if enabled {
                assert_parallel_membership_initplan(&plan);
            }
            if run == 11 {
                println!("organization count RLS={enabled} parallel={parallel}\n{plan}");
            }
            if run != 0 {
                times.push(execution_ms(&plan)?);
            }
        }
    }
    for times in &mut samples {
        times.sort_by(f64::total_cmp);
    }
    let baseline = samples[0][5];
    let serial = samples[1][5];
    let secured = samples[2][5];
    println!(
        "organization count: baseline median={baseline:.3} ms, serial baseline median={serial:.3} ms, RLS median={secured:.3} ms, ratio={:.3}",
        secured / baseline
    );
    assert!(secured <= 3.0 * baseline, "organization count exceeds 3x");
    Ok(())
}

fn assert_parallel_membership_initplan(plan: &str) {
    let lines: Vec<_> = plan.lines().collect();
    let init = lines
        .iter()
        .position(|line| line.trim_start().starts_with("InitPlan "));
    let gather = lines.iter().position(|line| line.contains("->  Gather "));
    let scan = lines
        .iter()
        .position(|line| line.contains("Parallel Seq Scan on messages"));
    let (Some(init), Some(gather), Some(scan)) = (init, gather, scan) else {
        panic!("membership InitPlan and parallel message scan required:\n{plan}");
    };
    assert!(init < gather && gather < scan, "{plan}");
    let indent = |line: &str| line.len() - line.trim_start().len();
    assert_eq!(indent(lines[init]), indent(lines[gather]), "{plan}");
    assert!(indent(lines[scan]) > indent(lines[gather]), "{plan}");
    assert!(
        lines[init + 1..gather]
            .iter()
            .any(|line| line.contains("Function Scan on fukulow_active_organizations")),
        "{plan}"
    );
    let reference = lines[init].trim().split(" (returns").next().unwrap();
    assert!(
        lines[scan + 1..].iter().any(|line| {
            line.contains("Filter:")
                && line.contains("organization_id = ANY")
                && line.contains(&format!("({reference}).col1"))
        }),
        "workers must filter with the leader's InitPlan result:\n{plan}"
    );
}
