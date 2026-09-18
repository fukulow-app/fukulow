#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Clippy's test options do not cover integration-test helper functions"
)]

#[path = "../src/invites.rs"]
mod invites;
mod support;

// Reuse the production lifecycle without exposing test routes in the application.
#[path = "../src/http.rs"]
mod http;
#[path = "../src/sessions.rs"]
mod sessions;
#[path = "../src/shutdown.rs"]
mod shutdown;
#[path = "../src/telemetry.rs"]
mod telemetry;

use std::{net::TcpListener, process::Command};

use anyhow::Result;
use support::AppProcess;

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_app"));
    command
        .env("FUKULOW_BIND_ADDR", "127.0.0.1:0")
        .env("RUST_LOG", "info")
        .env("FUKULOW_PUBLIC_ORIGIN", "http://localhost:8080")
        .env_remove("INSPECTOR_DATABASE_URL");
    command
}

#[test]
fn health_has_the_expected_status_content_type_and_body() -> Result<()> {
    let app = AppProcess::spawn(command())?;
    support::health(app.address()?)
}

#[test]
fn invalid_bind_address_fails_without_disclosing_the_value() -> Result<()> {
    let rejected = "not-a-socket-address";
    let mut command = command();
    command.env("FUKULOW_BIND_ADDR", rejected);
    let mut app = AppProcess::spawn_configured(command)?;
    let status = app.wait_for_exit()?;
    let logs = app.remaining_logs();
    assert!(!status.success());
    assert!(logs.contains("FUKULOW_BIND_ADDR"), "{logs}");
    assert!(!logs.contains(rejected), "rejected value appeared in logs");
    Ok(())
}

#[test]
fn occupied_address_fails_with_an_address_in_use_message() -> Result<()> {
    let occupied = TcpListener::bind("127.0.0.1:0")?;
    let mut command = command();
    command.env("FUKULOW_BIND_ADDR", occupied.local_addr()?.to_string());
    let mut app = AppProcess::spawn_configured(command)?;
    let status = app.wait_for_exit()?;
    let logs = app.remaining_logs();
    assert!(!status.success());
    assert!(logs.contains("address is in use"), "{logs}");
    Ok(())
}

#[test]
fn invalid_log_filter_warns_and_starts_with_info() -> Result<()> {
    let mut command = command();
    command.env("RUST_LOG", "info,app=not-a-level");
    let app = AppProcess::spawn_configured(command)?;
    let warning = app.wait_for_log("Invalid RUST_LOG")?;
    assert!(warning.contains("WARN"), "{warning}");
    assert!(warning.contains("info"), "{warning}");
    assert!(!warning.contains("not-a-level"));
    support::health(app.address()?)
}

#[test]
fn missing_log_filter_defaults_to_info() -> Result<()> {
    let mut command = command();
    command.env_remove("RUST_LOG");
    let app = AppProcess::spawn_configured(command)?;
    support::health(app.address()?)
}

#[cfg(unix)]
#[test]
fn signals_exit_the_application_successfully() -> Result<()> {
    for signal in ["-INT", "-TERM"] {
        let mut app = AppProcess::spawn(command())?;
        support::health(app.address()?)?;
        app.signal(signal)?;
        assert!(app.wait_for_exit()?.success(), "{signal}");
    }
    Ok(())
}

#[test]
#[ignore = "subprocess fixture for the shutdown tests"]
fn held_request_process() -> Result<()> {
    telemetry::init()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let pool = db::connect(&std::env::var("DATABASE_URL")?).await?;
        let routes = http::routes(sessions::StateData::new(
            pool,
            "http://localhost:8080".into(),
        ))
        .route("/hold", axum::routing::post(held_request));
        http::run(([127, 0, 0, 1], 0).into(), routes).await
    })
}

async fn held_request(body: axum::body::Body) -> Result<&'static str, axum::http::StatusCode> {
    tracing::info!("Held request entered");
    match axum::body::to_bytes(body, 1).await {
        Ok(body) if body.as_ref() == b"x" => Ok("completed"),
        _ => Err(axum::http::StatusCode::BAD_REQUEST),
    }
}

#[cfg(unix)]
fn assert_graceful_shutdown(signal: &str) -> Result<()> {
    use std::io::{Read, Write};

    let mut command = Command::new(std::env::current_exe()?);
    command.args([
        "--exact",
        "held_request_process",
        "--ignored",
        "--nocapture",
    ]);
    let mut app = AppProcess::spawn(command)?;
    let address = app.address()?;
    support::health(address)?;
    let mut stream = support::connect(address)?;
    stream.write_all(
        b"POST /hold HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1\r\nConnection: close\r\n\r\n",
    )?;
    // These acknowledgements put the signal strictly inside the request lifetime.
    app.wait_for_log("Held request entered")?;
    app.signal(signal)?;
    app.wait_for_log("Shutdown signal received")?;
    stream.write_all(b"x")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert_eq!(
        response.split_once("\r\n\r\n").map(|(_, body)| body),
        Some("completed")
    );
    assert!(app.wait_for_exit()?.success(), "{signal}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn sigint_drains_an_in_flight_request() -> Result<()> {
    assert_graceful_shutdown("-INT")
}

#[cfg(unix)]
#[test]
fn sigterm_drains_an_in_flight_request() -> Result<()> {
    assert_graceful_shutdown("-TERM")
}

#[test]
fn missing_database_url_fails_naming_only_the_variable() -> Result<()> {
    let mut command = command();
    command.env_remove("DATABASE_URL");
    let mut app = AppProcess::spawn_configured(command)?;
    assert!(!app.wait_for_exit()?.success());
    assert!(app.remaining_logs().contains("DATABASE_URL"));
    Ok(())
}

#[test]
fn invalid_database_url_fails_without_disclosing_the_value() -> Result<()> {
    let rejected = "invalid-database-url-fixture";
    let mut command = command();
    command.env("DATABASE_URL", rejected);
    let mut app = AppProcess::spawn_configured(command)?;
    assert!(!app.wait_for_exit()?.success());
    let logs = app.remaining_logs();
    assert!(logs.contains("DATABASE_URL"));
    assert!(!logs.contains(rejected));
    Ok(())
}

#[test]
fn non_postgres_listener_fails_the_startup_round_trip() -> Result<()> {
    use std::io::Write;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    // A reserved fixture marker, not a credential. gitleaks:allow
    const MARKER: &str = "database-credential-fixture";
    let url = format!(
        "postgres://fixture:{MARKER}@{}/fixture",
        listener.local_addr()?
    );
    let mut command = command();
    command.env("DATABASE_URL", &url);
    let mut app = AppProcess::spawn_configured(command)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(b"not PostgreSQL");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!app.wait_for_exit()?.success());
    let logs = app.remaining_logs();
    assert!(logs.contains("DATABASE_URL"));
    assert!(!logs.contains(&url));
    assert!(!logs.contains(MARKER));
    Ok(())
}

#[test]
fn inspector_configuration_refuses_startup_without_disclosing_its_value() -> Result<()> {
    let mut command = command();
    command.env("INSPECTOR_DATABASE_URL", "inspection-fixture-marker");
    let mut app = AppProcess::spawn_configured(command)?;
    assert!(!app.wait_for_exit()?.success());
    let logs = app.remaining_logs();
    assert!(logs.contains("INSPECTOR_DATABASE_URL"));
    assert!(!logs.contains("inspection-fixture-marker"));
    Ok(())
}

#[test]
fn public_origin_is_required_and_invalid_values_are_not_disclosed() -> Result<()> {
    let mut missing = command();
    missing.env_remove("FUKULOW_PUBLIC_ORIGIN");
    let mut app = AppProcess::spawn_configured(missing)?;
    assert!(!app.wait_for_exit()?.success());
    assert!(
        app.remaining_logs()
            .contains("FUKULOW_PUBLIC_ORIGIN is required")
    );
    for rejected in [
        "http://192.0.2.1",
        "http://chat.example.invalid",
        "ftp://localhost",
        "https://chat.example.invalid/",
        "https://chat.example.invalid/path",
        "https://chat.example.invalid?query",
        "https://chat.example.invalid#fragment",
        "https://fixture@chat.example.invalid",
        "https://chat.example.invalid:invalid",
        "https://chat.example.invalid:65536",
        "https://",
        "not-an-origin",
        // Non-canonical forms a browser never sends: the Origin check compares bytes.
        "https://Chat.Example.Invalid",
        "HTTPS://chat.example.invalid",
        "https://chat.example.invalid:443",
        "http://LOCALHOST:8080",
        "http://localhost:80",
        "https://chat.example.invalid:08443",
        "http://localhost:0080",
    ] {
        let mut command = command();
        command.env("FUKULOW_PUBLIC_ORIGIN", rejected);
        let mut app = AppProcess::spawn_configured(command)?;
        assert!(!app.wait_for_exit()?.success());
        let logs = app.remaining_logs();
        assert!(logs.contains("FUKULOW_PUBLIC_ORIGIN"));
        assert!(
            logs.contains("must contain")
                || logs.contains("requires HTTPS")
                || logs.contains("must be written as a browser sends it")
        );
        assert!(!logs.contains(rejected));
    }
    Ok(())
}

#[test]
fn secure_and_loopback_public_origins_start() -> Result<()> {
    for accepted in [
        "https://chat.example.invalid",
        "https://chat.example.invalid:8443",
        "http://localhost",
        "http://localhost:8080",
        "http://127.0.0.1:8080",
        "http://[::1]:8080",
    ] {
        let mut command = command();
        command.env("FUKULOW_PUBLIC_ORIGIN", accepted);
        let app = AppProcess::spawn_configured(command)?;
        support::health(app.address()?)?;
    }
    Ok(())
}

#[test]
fn session_routes_reject_before_handlers() -> Result<()> {
    use std::io::{Read, Write};
    let app = AppProcess::spawn(command())?;
    let address = app.address()?;
    let mut statuses = Vec::new();
    for (method, path, origin, body) in [
        ("GET", "/api/v1/me", "", ""),
        ("POST", "/api/v1/sessions", "", "{}"),
        (
            "POST",
            "/api/v1/sessions",
            "Origin: http://localhost:8080\r\n",
            "{}",
        ),
    ] {
        let mut stream = support::connect(address)?;
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{origin}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        statuses.push(response.split_whitespace().nth(1).unwrap().to_owned());
        assert!(response.split_once("\r\n\r\n").unwrap().1.is_empty());
    }
    assert_eq!(statuses, ["401", "403", "422"]);
    Ok(())
}

#[test]
fn unrouted_state_changing_requests_check_origin_first() -> Result<()> {
    use std::io::{Read, Write};
    let app = AppProcess::spawn(command())?;
    let address = app.address()?;
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        for (origin, expected) in [
            ("", "403"),
            ("Origin: https://foreign.example.invalid\r\n", "403"),
            ("Origin: http://localhost:8080\r\n", "404"),
        ] {
            let mut stream = support::connect(address)?;
            write!(
                stream,
                "{method} /api/v1/does-not-exist HTTP/1.1\r\nHost: localhost\r\n{origin}Content-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
            let mut response = String::new();
            stream.read_to_string(&mut response)?;
            assert_eq!(
                response.split_whitespace().nth(1),
                Some(expected),
                "{method} with {origin:?}"
            );
            assert!(
                response.split_once("\r\n\r\n").unwrap().1.is_empty(),
                "{method} with {origin:?}"
            );
        }
    }
    Ok(())
}
