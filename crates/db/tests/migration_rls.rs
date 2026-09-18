#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Clippy's test options do not cover integration-test helper functions"
)]

use std::{collections::BTreeSet, fs, path::Path};

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    Identifier(String),
    Literal,
    Symbol(u8),
}

impl Token {
    fn name(&self) -> Option<&str> {
        match self {
            Self::Word(name) | Self::Identifier(name) => Some(name),
            _ => None,
        }
    }

    fn keyword(&self, word: &str) -> bool {
        matches!(self, Self::Word(name) if name == word)
    }
}

fn starts(tokens: &[Token], words: &[&str]) -> bool {
    tokens.len() >= words.len()
        && tokens
            .iter()
            .zip(words)
            .all(|(token, word)| token.keyword(word))
}

fn block_comment(sql: &str, mut position: usize) -> Result<usize> {
    let mut depth = 1;
    position += 2;
    while position < sql.len() {
        if sql[position..].starts_with("/*") {
            depth += 1;
            position += 2;
        } else if sql[position..].starts_with("*/") {
            depth -= 1;
            position += 2;
            if depth == 0 {
                return Ok(position);
            }
        } else {
            position += sql[position..].chars().next().unwrap().len_utf8();
        }
    }
    Err("unterminated block comment".into())
}

fn quoted(sql: &str, start: usize, escape: bool) -> Result<usize> {
    let bytes = sql.as_bytes();
    let delimiter = bytes[start];
    let mut position = start + 1;
    while position < bytes.len() {
        if bytes[position] == delimiter {
            if bytes.get(position + 1) == Some(&delimiter) {
                position += 2;
            } else {
                return Ok(position + 1);
            }
        } else if escape && bytes[position] == b'\\' {
            position += 2;
        } else {
            position += 1;
        }
    }
    Err("unterminated quoted text".into())
}

fn dollar_body(sql: &str, start: usize) -> Result<usize> {
    let rest = &sql[start + 1..];
    let length = rest.find('$').ok_or("unterminated dollar delimiter")?;
    let tag = &rest[..length];
    if !tag.bytes().enumerate().all(|(index, byte)| {
        byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
    }) {
        return Err("unsupported dollar delimiter".into());
    }
    let delimiter = &sql[start..start + length + 2];
    let body = start + delimiter.len();
    let end = sql[body..]
        .find(delimiter)
        .ok_or("unterminated dollar body")?;
    Ok(body + end + delimiter.len())
}

fn tokens(sql: &str) -> Result<Vec<Token>> {
    let bytes = sql.as_bytes();
    let mut result = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        let start = position;
        let rest = &sql[position..];
        let byte = bytes[position];
        if byte.is_ascii_whitespace() {
            position += 1;
        } else if rest.starts_with("--") {
            position += rest.find('\n').unwrap_or(rest.len());
            if sql[start + 2..position]
                .trim()
                .starts_with("no-transaction")
            {
                return Err("sqlx no-transaction directive is forbidden".into());
            }
        } else if rest.starts_with("/*") {
            position = block_comment(sql, position)?;
        } else if rest.starts_with("U&") || rest.starts_with("u&") {
            return Err("unsupported Unicode escape quoting".into());
        } else if byte == b'\'' || byte == b'"' {
            position = quoted(sql, position, false)?;
            result.push(if byte == b'"' {
                Token::Identifier(sql[start + 1..position - 1].replace("\"\"", "\""))
            } else {
                Token::Literal
            });
        } else if (byte == b'e' || byte == b'E') && bytes.get(position + 1) == Some(&b'\'') {
            position = quoted(sql, position + 1, true)?;
            result.push(Token::Literal);
        } else if byte == b'$' {
            position = dollar_body(sql, position)?;
            result.push(Token::Literal);
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            position += 1;
            while bytes
                .get(position)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$')
            {
                position += 1;
            }
            result.push(Token::Word(sql[start..position].to_ascii_lowercase()));
        } else if byte.is_ascii() {
            position += 1;
            result.push(Token::Symbol(byte));
        } else {
            return Err("unsupported unquoted character".into());
        }
    }
    Ok(result)
}

fn statements(sql: &str) -> Result<Vec<Vec<Token>>> {
    let mut result = Vec::new();
    let mut statement = Vec::new();
    let mut depth = 0_usize;
    for token in tokens(sql)? {
        match token {
            Token::Symbol(b'(') => depth += 1,
            Token::Symbol(b')') => depth = depth.checked_sub(1).ok_or("unmatched parenthesis")?,
            Token::Symbol(b';') => {
                if depth != 0 {
                    return Err("semicolon inside unquoted parentheses".into());
                }
                if !statement.is_empty() {
                    result.push(std::mem::take(&mut statement));
                }
                continue;
            }
            _ => {}
        }
        statement.push(token);
    }
    if depth != 0 {
        return Err("unclosed parenthesis".into());
    }
    if !statement.is_empty() {
        result.push(statement);
    }
    Ok(result)
}

// Unqualified names use public in these migrations. Other schemas remain distinct,
// so relaxing one table cannot accidentally authorize another with the same name.
fn table_name(tokens: &[Token]) -> Result<(String, &[Token])> {
    let mut tokens = tokens;
    if starts(tokens, &["if", "exists"]) {
        tokens = &tokens[2..];
    }
    if starts(tokens, &["only"]) {
        tokens = &tokens[1..];
    }
    let first = tokens
        .first()
        .and_then(Token::name)
        .ok_or("missing table name")?;
    if first.contains('.') {
        return Err("unsupported dot inside table identifier".into());
    }
    if tokens.get(1) == Some(&Token::Symbol(b'.')) {
        let second = tokens
            .get(2)
            .and_then(Token::name)
            .ok_or("missing qualified table name")?;
        if second.contains('.') {
            return Err("unsupported dot inside table identifier".into());
        }
        Ok((format!("{first}.{second}"), &tokens[3..]))
    } else {
        Ok((format!("public.{first}"), &tokens[1..]))
    }
}

fn rls_action(statement: &[Token]) -> Result<Option<(String, &str)>> {
    if !starts(statement, &["alter", "table"]) {
        return Ok(None);
    }
    let (table, tail) = table_name(&statement[2..])?;
    for (words, action) in [
        (&["enable", "row", "level", "security"][..], "enable"),
        (&["disable", "row", "level", "security"][..], "disable"),
        (&["no", "force", "row", "level", "security"][..], "no force"),
        (&["force", "row", "level", "security"][..], "force"),
    ] {
        if starts(tail, words) && tail.len() == words.len() {
            return Ok(Some((table, action)));
        }
    }
    // Combined ALTER actions and renames would need a richer table/state model.
    if tail.iter().any(|token| {
        token.keyword("security") || token.keyword("rename") || token.keyword("schema")
    }) {
        return Err("unsupported ALTER TABLE row-security or identity change".into());
    }
    Ok(None)
}

fn created_table(statement: &[Token]) -> Result<Option<String>> {
    if !starts(statement, &["create"]) {
        return Ok(None);
    }
    let mut tail = &statement[1..];
    if tail.first().is_some_and(|token| {
        token.keyword("unlogged") || token.keyword("temporary") || token.keyword("temp")
    }) {
        tail = &tail[1..];
    }
    if !starts(tail, &["table"]) {
        return Ok(None);
    }
    tail = &tail[1..];
    if starts(tail, &["if", "not", "exists"]) {
        tail = &tail[3..];
    }
    Ok(Some(table_name(tail)?.0))
}

struct Migration {
    name: String,
    statements: Vec<Vec<Token>>,
}

fn protected_tables(migrations: &[Migration]) -> Result<BTreeSet<String>> {
    let mut tables = BTreeSet::new();
    for migration in migrations {
        for statement in &migration.statements {
            if let Some((table, "enable")) = rls_action(statement)? {
                tables.insert(table);
            }
        }
    }
    Ok(tables)
}

fn check_migration(migration: &Migration, tables: &BTreeSet<String>) -> Result<()> {
    let mut open = BTreeSet::new();
    let mut removed = BTreeSet::new();
    for (index, statement) in migration.statements.iter().enumerate() {
        let result = check_statement(statement, tables, &mut open, &mut removed);
        result.map_err(|error| format!("{}: statement {}: {error}", migration.name, index + 1))?;
    }
    for table in open {
        if !(migration.name.ends_with(".down.sql") && removed.contains(&table)) {
            return Err(format!("{}: {table} left in NO FORCE", migration.name));
        }
    }
    Ok(())
}

fn check_statement(
    statement: &[Token],
    tables: &BTreeSet<String>,
    open: &mut BTreeSet<String>,
    removed: &mut BTreeSet<String>,
) -> Result<()> {
    let kind = match statement.first() {
        Some(Token::Word(kind)) => kind.as_str(),
        _ => return Err("unclassified statement".into()),
    };
    // New statement kinds must be admitted here deliberately. WITH is unsupported,
    // including data-changing CTEs; DO and CALL hide immediately executed SQL.
    match kind {
        "create" | "alter" | "drop" | "comment" | "grant" | "revoke" => {}
        "insert" | "update" | "delete" => {
            // Conservatively count every identifier matching a protected table,
            // including nested reads, joins and comma lists. An alias/column with
            // the same name can require an extra bracket; it cannot hide a read.
            for table in tables {
                let name = table.rsplit('.').next().unwrap();
                if statement.iter().any(|token| token.name() == Some(name))
                    && (!open.contains(table) || removed.contains(table))
                {
                    return Err(format!(
                        "{table} requires NO FORCE before its data statement"
                    ));
                }
            }
        }
        _ => return Err(format!("unclassified or forbidden statement: {kind}")),
    }
    if let Some((table, action)) = rls_action(statement)? {
        match action {
            "no force" => {
                open.insert(table);
            }
            "force" => {
                open.remove(&table);
            }
            "disable" => {
                removed.insert(table);
            }
            "enable" => {
                removed.remove(&table);
            }
            _ => unreachable!(),
        }
    }
    if starts(statement, &["drop", "table"]) {
        let (table, tail) = table_name(&statement[2..])?;
        if !tail.is_empty()
            && !(tail.len() == 1 && (tail[0].keyword("cascade") || tail[0].keyword("restrict")))
        {
            return Err("unsupported DROP TABLE form".into());
        }
        removed.insert(table);
    }
    if let Some(table) = created_table(statement)? {
        removed.remove(&table);
    }
    // CREATE ... AS can execute queries; only known deferred definitions are exempt.
    if kind == "create"
        && statement
            .iter()
            .any(|token| token.keyword("select") || token.keyword("as"))
        && !starts(statement, &["create", "function"])
        && !starts(statement, &["create", "or", "replace", "function"])
        && !starts(statement, &["create", "policy"])
    {
        return Err("unsupported CREATE with an executable query".into());
    }
    Ok(())
}

#[test]
fn migrations_bracket_row_security() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    let migrations: Vec<_> = paths
        .iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            assert!(
                name.ends_with(".up.sql") || name.ends_with(".down.sql"),
                "unexpected migration file: {name}"
            );
            let sql = fs::read_to_string(path).unwrap();
            let statements = statements(&sql).unwrap_or_else(|error| panic!("{name}: {error}"));
            Migration { name, statements }
        })
        .collect();
    assert!(!migrations.is_empty(), "no migrations found");
    let tables = protected_tables(&migrations).unwrap();
    assert!(!tables.is_empty(), "no row-security tables found");
    for migration in &migrations {
        check_migration(migration, &tables).unwrap_or_else(|error| panic!("{error}"));
    }
}

fn probe(sql: &str, down: bool) -> Result<()> {
    let schema = Migration {
        name: "schema.up.sql".into(),
        statements: statements(
            "ALTER TABLE messages ENABLE ROW LEVEL SECURITY; ALTER TABLE channels ENABLE ROW LEVEL SECURITY;",
        )?,
    };
    let migration = Migration {
        name: if down {
            "probe.down.sql"
        } else {
            "probe.up.sql"
        }
        .into(),
        statements: statements(sql)?,
    };
    check_migration(&migration, &protected_tables(&[schema])?)
}

#[test]
fn missing_brackets_and_hidden_execution_are_refused() {
    for (sql, error) in [
        (
            "UPDATE messages SET channel_seq = 1;",
            "public.messages requires NO FORCE",
        ),
        (
            "INSERT INTO messages DEFAULT VALUES;",
            "public.messages requires NO FORCE",
        ),
        ("DELETE FROM messages;", "public.messages requires NO FORCE"),
        (
            "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; INSERT INTO messages SELECT * FROM channels; ALTER TABLE messages FORCE ROW LEVEL SECURITY;",
            "public.channels requires NO FORCE",
        ),
        (
            "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY;",
            "left in NO FORCE",
        ),
        (
            "DO $$ BEGIN UPDATE messages SET channel_seq = 1; END $$;",
            "forbidden statement: do",
        ),
        (
            "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; COMMIT; ALTER TABLE messages FORCE ROW LEVEL SECURITY;",
            "forbidden statement: commit",
        ),
        (
            "-- no-transaction\nCREATE TABLE fixture (id int);",
            "no-transaction",
        ),
        ("CALL fixture();", "forbidden statement: call"),
        ("SELECT * FROM messages;", "forbidden statement: select"),
        ("COPY messages TO STDOUT;", "forbidden statement: copy"),
        ("BEGIN;", "forbidden statement: begin"),
        ("ROLLBACK;", "forbidden statement: rollback"),
        ("SAVEPOINT fixture;", "forbidden statement: savepoint"),
        ("TRUNCATE messages;", "forbidden statement: truncate"),
        (
            "WITH changed AS (DELETE FROM messages RETURNING *) SELECT * FROM changed;",
            "forbidden statement: with",
        ),
        (
            "CREATE TABLE fixture AS SELECT * FROM messages;",
            "executable query",
        ),
        (
            "ALTER TABLE messages ADD COLUMN fixture int, NO FORCE ROW LEVEL SECURITY;",
            "unsupported ALTER TABLE",
        ),
    ] {
        let actual = probe(sql, false).expect_err(sql);
        assert!(actual.contains(error), "{sql}: {actual}");
    }
}

#[test]
fn reads_in_joins_subqueries_and_qualified_names_need_brackets() {
    for sql in [
        "UPDATE messages SET channel_seq = c.next_message_seq FROM channels c;",
        "DELETE FROM messages USING channels WHERE messages.channel_id = channels.id;",
        "INSERT INTO messages SELECT m.* FROM messages m, channels c;",
        "UPDATE messages SET channel_seq = (SELECT next_message_seq FROM channels);",
        "INSERT INTO messages SELECT * FROM ONLY public.channels;",
        "INSERT INTO messages SELECT * FROM \"public\".\"channels\";",
        "INSERT INTO messages SELECT * FROM channels JOIN messages ON true;",
    ] {
        let destination_only = format!(
            "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; {sql} ALTER TABLE messages FORCE ROW LEVEL SECURITY;"
        );
        assert!(
            probe(&destination_only, false)
                .unwrap_err()
                .contains("public.channels requires NO FORCE")
        );
        let both = format!(
            "ALTER TABLE channels NO FORCE ROW LEVEL SECURITY; {destination_only} ALTER TABLE channels FORCE ROW LEVEL SECURITY;"
        );
        probe(&both, false).unwrap();
    }
    assert!(probe("ALTER TABLE other.messages NO FORCE ROW LEVEL SECURITY; UPDATE public.messages SET channel_seq = 1; ALTER TABLE other.messages FORCE ROW LEVEL SECURITY;", false).is_err());
}

#[test]
fn comments_and_quoted_bodies_do_not_execute() {
    for sql in [
        "CREATE FUNCTION fixture() RETURNS void LANGUAGE plpgsql AS $$ BEGIN UPDATE messages SET channel_seq = 1; END $$;",
        "CREATE OR REPLACE FUNCTION fixture() RETURNS void LANGUAGE plpgsql AS $body_1$ BEGIN UPDATE messages SET channel_seq = 1; PERFORM '$$'; END $body_1$;",
        "CREATE FUNCTION fixture() RETURNS void LANGUAGE sql AS 'UPDATE messages SET channel_seq = 1;';",
        "COMMENT ON TABLE messages IS 'it''s UPDATE messages; -- no-transaction';",
        r"COMMENT ON TABLE messages IS E'it\'s UPDATE messages;';",
        "/* outer UPDATE messages; /* inner */ COMMIT; */ -- DELETE FROM messages;\nCREATE TABLE fixture (id int);",
    ] {
        probe(sql, false).unwrap();
    }
    for sql in [
        "CREATE FUNCTION fixture() RETURNS void LANGUAGE sql AS $$ SELECT 1; $$; UPDATE messages SET channel_seq = 1;",
        "/* UPDATE channels; */ UPDATE messages SET channel_seq = 1;",
        "COMMENT ON TABLE messages IS 'FORCE ROW LEVEL SECURITY;'; UPDATE messages SET channel_seq = 1;",
    ] {
        assert!(probe(sql, false).is_err(), "{sql}");
    }
}

#[test]
fn only_down_migrations_can_omit_restoration_after_schema_removal() {
    for ending in [
        "ALTER TABLE messages DISABLE ROW LEVEL SECURITY;",
        "DROP TABLE messages;",
    ] {
        let sql = format!("ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; {ending}");
        probe(&sql, true).unwrap();
        assert!(probe(&sql, false).unwrap_err().contains("left in NO FORCE"));
        assert!(probe(&format!("{sql} UPDATE messages SET channel_seq = 1;"), true).is_err());
    }
    for sql in [
        "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY;",
        "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; DROP TABLE channels;",
        "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; ALTER TABLE messages DISABLE ROW LEVEL SECURITY; ALTER TABLE messages ENABLE ROW LEVEL SECURITY;",
        "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; DROP TABLE messages; CREATE TABLE messages (id int);",
        "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; DROP TABLE messages; CREATE TABLE IF NOT EXISTS messages (id int);",
        "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; DROP TABLE messages; CREATE UNLOGGED TABLE messages (id int);",
    ] {
        assert!(probe(sql, true).unwrap_err().contains("left in NO FORCE"));
    }
}

#[test]
fn brackets_are_local_ordered_and_identifier_aware() {
    for sql in [
        "ALTER TABLE messages FORCE ROW LEVEL SECURITY; UPDATE messages SET channel_seq = 1; ALTER TABLE messages NO FORCE ROW LEVEL SECURITY;",
        "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; ALTER TABLE messages FORCE ROW LEVEL SECURITY; DELETE FROM messages;",
        "ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; ALTER TABLE channels FORCE ROW LEVEL SECURITY;",
    ] {
        assert!(probe(sql, false).is_err(), "{sql}");
    }
    probe("aLtEr TABLE ONLY \"public\".\"messages\" NO FORCE ROW LEVEL SECURITY; UPDATE PUBLIC.MESSAGES SET channel_seq = 1; ALTER TABLE messages FORCE ROW LEVEL SECURITY", false).unwrap();
    probe("ALTER TABLE messages NO FORCE ROW LEVEL SECURITY; UPDATE messages SET channel_seq = 1; ALTER TABLE messages FORCE ROW LEVEL SECURITY;", false).unwrap();
    assert!(probe("UPDATE messages SET channel_seq = 1;", false).is_err());
}

#[test]
fn malformed_or_unclassified_text_is_refused() {
    for sql in [
        "/* unfinished",
        "COMMENT ON TABLE messages IS 'unfinished",
        "DO $tag$unfinished",
        "ALTER TABLE \"unfinished",
        "CREATE TABLE fixture (id int;",
        "CREATE TABLE fixture (id int",
        "$$UPDATE messages;$$;",
        "'UPDATE messages;';",
        "\"update\" messages;",
        "VACUUM messages;",
    ] {
        assert!(probe(sql, false).is_err(), "{sql}");
    }
}
