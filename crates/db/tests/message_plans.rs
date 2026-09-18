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
use support::{Conversation, Database, Result};
use uuid::Uuid;

#[tokio::test]
async fn history_plans_walk_sequence_index_and_stop_at_limit() -> Result {
    let db = Database::new().await?;
    let c = Conversation::new(&db).await?;
    let ids: Vec<_> = (0..100_000).map(|_| Uuid::now_v7()).collect();
    sqlx::query("INSERT INTO messages (id, organization_id, channel_id, channel_seq, sender_actor_id, body) SELECT id, $1, $2, seq, $3, 'Fixture body' FROM unnest($4::uuid[]) WITH ORDINALITY AS rows(id, seq)")
        .bind(c.organization.0).bind(c.channel.0).bind(c.owner.0).bind(ids).execute(&db.inspector).await?;
    sqlx::query(
        "UPDATE channels SET next_message_seq = 100000 WHERE organization_id = $1 AND id = $2",
    )
    .bind(c.organization.0)
    .bind(c.channel.0)
    .execute(&db.inspector)
    .await?;
    sqlx::query("ANALYZE messages").execute(&db.owner).await?;
    // Read the exact production literals so a query change cannot leave this evidence stale.
    let source = include_str!("../src/operations/messages.rs");
    let queries: Vec<_> = source
        .split('"')
        .filter(|s| s.starts_with("SELECT id, channel_id,") && s.contains("ORDER BY channel_seq"))
        .collect();
    assert_eq!(queries.len(), 3);
    for query in queries {
        let mut connection = db.actor(c.owner).await?;
        // EXPLAIN cannot be bound around a statement. Only these source-owned, static literals are accepted.
        let explain = match query {
            "SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 AND channel_seq < $3 ORDER BY channel_seq DESC LIMIT $4" => {
                "EXPLAIN (ANALYZE, BUFFERS) SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 AND channel_seq < $3 ORDER BY channel_seq DESC LIMIT $4"
            }
            "SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 ORDER BY channel_seq DESC LIMIT $3" => {
                "EXPLAIN (ANALYZE, BUFFERS) SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 ORDER BY channel_seq DESC LIMIT $3"
            }
            "SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 AND channel_seq > $3 ORDER BY channel_seq ASC LIMIT $4" => {
                "EXPLAIN (ANALYZE, BUFFERS) SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 AND channel_seq > $3 ORDER BY channel_seq ASC LIMIT $4"
            }
            _ => return Err("history query changed: update its EXPLAIN literal".into()),
        };
        let statement = sqlx::query_scalar::<_, String>(explain)
            .bind(c.organization.0)
            .bind(c.channel.0);
        let plan = if query.contains("LIMIT $4") {
            statement
                .bind(50_000_i64)
                .bind(100_i64)
                .fetch_all(&mut *connection)
                .await?
        } else {
            statement.bind(100_i64).fetch_all(&mut *connection).await?
        }
        .join("\n");
        println!("{query}\n{plan}\n");
        assert!(
            plan.contains("Index Scan") && plan.contains("messages_channel_id_channel_seq_key"),
            "{plan}"
        );
        assert!(!plan.contains("Sort") && !plan.contains("Bitmap"), "{plan}");
        assert!(plan.contains("rows=100 loops=1"), "{plan}");
    }
    db.finish().await
}
