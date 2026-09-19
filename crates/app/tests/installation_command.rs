#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Clippy's test options do not cover integration-test helper functions"
)]

#[path = "../../db/tests/support/mod.rs"]
mod database;

use sqlx::ConnectOptions;
use std::process::{Command, Stdio};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fukulow"));
    command
        .arg("bootstrap")
        .env_remove("MIGRATOR_DATABASE_URL")
        .env_remove("INSPECTOR_DATABASE_URL")
        .env_remove("FUKULOW_PUBLIC_ORIGIN")
        .env_remove("FUKULOW_BIND_ADDR")
        .stdin(Stdio::piped());
    command
}

#[tokio::test]
async fn bootstrap_refuses_piped_input_without_server_configuration() -> database::Result {
    let db = database::Database::new().await?;
    let before = database::installation::counts(&db).await?;
    let url = db.app.connect_options().to_url_lossy().to_string();
    let output =
        tokio::task::spawn_blocking(move || command().env("DATABASE_URL", url).output()).await??;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let output = String::from_utf8(output.stderr)?;
    assert!(output.contains("terminal"), "{output}");
    assert_eq!(database::installation::counts(&db).await?, before);
    db.finish().await
}

#[test]
fn bootstrap_refuses_the_inspector() {
    let output = command()
        .env("INSPECTOR_DATABASE_URL", "inspection-fixture-marker")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let output = String::from_utf8(output.stderr).unwrap();
    assert!(output.contains("INSPECTOR_DATABASE_URL"), "{output}");
    assert!(!output.contains("inspection-fixture-marker"));
}

#[test]
fn help_lists_bootstrap() {
    let output = Command::new(env!("CARGO_BIN_EXE_fukulow"))
        .arg("help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("bootstrap")
    );
}
