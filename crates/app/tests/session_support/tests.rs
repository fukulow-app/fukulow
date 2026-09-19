use super::*;
use std::{process::Command, time::Duration};

const CHILD: &str = "session_support::tests::leak_probe";

#[test]
#[should_panic(
    expected = "a value shorter than 16 characters can appear in unrelated log text; it is not a checkable secret"
)]
fn registration_refuses_short_values_before_locking() {
    register_secret("a@b");
}

#[test]
fn the_boundary_is_sixteen_characters_not_bytes() {
    assert!(!distinctive("fifteen-chars!!"));
    assert!(distinctive("sixteen-chars!!!"));
    assert!(distinctive("fixture-password"));
    // Sixteen bytes but eight characters: still too short.
    assert!(!distinctive(&"é".repeat(8)));
    assert!(distinctive(&"é".repeat(16)));
}

#[test]
fn automatic_scans_detect_leaks_without_explicit_checks() {
    // A deliberate leak must not contaminate the permanent registry of the parent binary.
    for scenario in [
        "clean",
        "request",
        "raw",
        "database_finish",
        "escaped",
        "concurrent",
        "session_failure",
        "invite_failure",
    ] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", CHILD, "--nocapture"])
            .env("FUKULOW_LEAK_PROBE", scenario)
            .env("RUST_BACKTRACE", "0")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("log registry subprocess timed out: {scenario}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        if scenario == "clean" {
            assert!(output.status.success(), "clean fixture failed");
        } else {
            assert!(!output.status.success(), "missed leak: {scenario}");
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("secret appeared in captured logs"),
                "probe failed for another reason: {scenario}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "isolates intentional leaks from the permanent per-binary registry"]
async fn leak_probe() -> Result {
    let scenario = std::env::var("FUKULOW_LEAK_PROBE")?;
    if scenario == "concurrent" {
        return concurrent_probe().await;
    }
    let db = database().await?;
    person(&db).await?;
    let mut state = state(db.app.clone());
    if scenario == "session_failure" {
        database::sessions::fail_session_insert(&db).await?;
        state.new_token = record_session;
    }
    if scenario == "invite_failure" {
        state.new_invite_token = record_invite;
    }
    let server = Server::new(state).await?;
    let response = server.sign_in().await?;
    if scenario == "session_failure" {
        response.assert_error(500);
        log_generated();
    } else if scenario == "invite_failure" {
        let cookie = response.cookie()?;
        let actor = db::resolve_session(&db.app, &auth::hash_session_token(&cookie)).await?;
        let org = db.organization(actor).await?;
        database::invites::fail_audit(&db).await?;
        server
            .request(
                "POST",
                &format!("/api/v1/organizations/{}/invites", org.0),
                Some(ORIGIN),
                Some(&cookie),
                r#"{"expires_in":300}"#,
            )
            .await?
            .assert_error(500);
        log_generated();
    } else if scenario != "clean" {
        // Includes characters whose Debug representation differs from their raw bytes.
        let value = format!("fixture-log-probe-é-{}\n\t\0\"\\", uuid::Uuid::now_v7());
        register_secret(&value);
        if scenario == "escaped" {
            tracing::info!(value = ?value.to_uppercase());
        } else {
            tracing::info!(value = %value.to_uppercase());
        }
    }
    if scenario == "raw" || scenario == "request" {
        let result = tokio::spawn(async move {
            if scenario == "raw" {
                server
                    .raw(
                        "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
                            .into(),
                    )
                    .await
            } else {
                server.request("GET", "/health", None, None, "").await
            }
        })
        .await;
        // Teardown still runs when the request scan panics; its own scan must not mask
        // a missing request check, so only the request's panic is propagated below.
        let _ = tokio::spawn(db.finish()).await;
        let Err(error) = result else {
            return Ok(());
        };
        assert!(error.is_panic());
        std::panic::resume_unwind(error.into_panic());
    }
    drop(server);
    db.finish().await
}

static GENERATED: Mutex<Option<String>> = Mutex::new(None);

fn record_session() -> std::result::Result<(auth::SessionToken, db::TokenHash), auth::TokenError> {
    let pair = new_session_token()?;
    *GENERATED.lock().unwrap() = Some(pair.0.as_str().to_owned());
    Ok(pair)
}

fn record_invite() -> std::result::Result<(auth::InviteToken, db::TokenHash), auth::TokenError> {
    let pair = new_invite_token()?;
    *GENERATED.lock().unwrap() = Some(pair.0.as_str().to_owned());
    Ok(pair)
}

fn log_generated() {
    let value = GENERATED.lock().unwrap().take().unwrap();
    tracing::info!(value = %value);
}

async fn concurrent_probe() -> Result {
    let (finished, after_finish) = tokio::sync::oneshot::channel();
    let ready = Arc::new(tokio::sync::Barrier::new(2));
    let first_ready = ready.clone();
    let first = tokio::spawn(async move {
        let db = database().await?;
        let value = format!("fixture-retained-{}", uuid::Uuid::now_v7());
        register_secret(&value);
        first_ready.wait().await;
        db.finish().await?;
        finished.send(value).unwrap();
        Result::Ok(())
    });
    let second = tokio::spawn(async move {
        let db = database().await?;
        ready.wait().await;
        let value = after_finish.await?;
        tracing::info!(value = %value);
        db.finish().await
    });
    // The parent's process deadline also bounds a synchronous registry deadlock, which
    // an async timeout alone cannot interrupt when the executor's workers are blocked.
    first.await??;
    let error = second
        .await
        .expect_err("another fixture missed the retained secret");
    assert!(error.is_panic());
    std::panic::resume_unwind(error.into_panic());
}
