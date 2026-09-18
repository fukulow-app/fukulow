mod support;

use domain::{ActorType, ChannelScope, MemberStatus, OrganizationRole, TeamId, TeamRole};
use std::collections::BTreeSet;
use support::{Database, Result};
use uuid::Uuid;

#[tokio::test]
async fn canonical_names_equal_database_checks() -> Result {
    let db = Database::new().await?;
    let enumerations = [
        (
            "actors",
            "type",
            ActorType::ALL.iter().map(|v| v.as_str()).collect(),
        ),
        (
            "organization_members",
            "role",
            OrganizationRole::ALL.iter().map(|v| v.as_str()).collect(),
        ),
        (
            "organization_members",
            "status",
            MemberStatus::ALL.iter().map(|v| v.as_str()).collect(),
        ),
        (
            "team_members",
            "role",
            TeamRole::ALL.iter().map(|v| v.as_str()).collect(),
        ),
        (
            "channels",
            "scope",
            BTreeSet::from([
                ChannelScope::Organization.as_str(),
                ChannelScope::Team(TeamId(Uuid::nil())).as_str(),
            ]),
        ),
    ];
    let mut checks = Vec::new();
    for (table, column, names) in enumerations {
        let definitions: Vec<String> = sqlx::query_scalar(
            "SELECT pg_get_constraintdef(k.oid) FROM pg_constraint k
             JOIN pg_class c ON c.oid = k.conrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(k.conkey)
             WHERE n.nspname = 'public' AND k.contype = 'c'
               AND c.relname = $1 AND a.attname = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_all(&db.owner)
        .await?;
        checks.push((table, column, names, definitions));
    }
    db.finish().await?;
    for (table, column, names, definitions) in checks {
        assert_eq!(definitions.len(), 1, "{table}.{column}");
        let definition = &definitions[0];
        let stored: BTreeSet<_> = if column == "scope" {
            // Scope is part of a table CHECK; unrelated literals must not count as scopes.
            definition
                .split("scope = '")
                .skip(1)
                .map(|comparison| comparison.split_once('\'').unwrap().0)
                .collect()
        } else {
            definition.split('\'').skip(1).step_by(2).collect()
        };
        assert_eq!(names, stored, "{table}.{column}");
    }
    Ok(())
}
