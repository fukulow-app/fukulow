use crate::{database, http, sessions};
use database::Result;
use std::{
    io::{Read, Write},
    net::SocketAddr,
    sync::{Arc, Mutex, OnceLock},
};

pub(crate) const ORIGIN: &str = "https://chat.example.invalid";
pub(crate) const EMAIL: &str = "fixture@example.invalid";
// A public test input, never a credential for a deployed account. gitleaks:allow
pub(crate) const PASSWORD: &str = "fixture-password";
// Matches the public dummy hash, so accepting its verification would be observable. gitleaks:allow
pub(crate) const DUMMY_PASSWORD: &str = "fixture-dummy-password";

pub(crate) async fn person(db: &database::Database) -> Result<domain::ActorId> {
    for value in [EMAIL, PASSWORD, "Fixture person"] {
        register_secret(value);
    }
    let hash = tokio::task::spawn_blocking(|| auth::hash_password(PASSWORD)).await??;
    let mut tx = db.app.begin().await?;
    let actor = db::create_person(&mut tx, EMAIL, &hash, "Fixture person").await?;
    tx.commit().await?;
    Ok(actor)
}

pub(crate) async fn database() -> Result<database::Database> {
    database::Database::with_secret_checks(register_secret, scan_logs).await
}

pub(crate) fn state(pool: sqlx::PgPool) -> sessions::StateData {
    let mut state = sessions::StateData::new(pool, ORIGIN.into());
    state.new_token = new_session_token;
    state.new_invite_token = new_invite_token;
    state
}

pub(crate) fn new_session_token()
-> std::result::Result<(auth::SessionToken, db::TokenHash), auth::TokenError> {
    let pair = auth::new_session_token()?;
    register_secret(pair.0.as_str());
    Ok(pair)
}

pub(crate) fn new_invite_token()
-> std::result::Result<(auth::InviteToken, db::TokenHash), auth::TokenError> {
    let pair = auth::new_invite_token()?;
    register_secret(pair.0.as_str());
    Ok(pair)
}

pub(crate) struct Server {
    address: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    pub(crate) async fn new(state: sessions::StateData) -> Result<Self> {
        logs();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            axum::serve(listener, http::routes(state)).await.unwrap();
        });
        Ok(Self { address, task })
    }

    pub(crate) async fn sign_in(&self) -> Result<Response> {
        self.request(
            "POST",
            "/api/v1/sessions",
            Some(ORIGIN),
            None,
            &serde_json::json!({"email":EMAIL.to_ascii_uppercase(),"password":PASSWORD})
                .to_string(),
        )
        .await
    }

    pub(crate) async fn request(
        &self,
        method: &str,
        path: &str,
        origin: Option<&str>,
        token: Option<&str>,
        body: &str,
    ) -> Result<Response> {
        let origin = origin
            .map(|origin| format!("Origin: {origin}\r\n"))
            .unwrap_or_default();
        let cookie = token
            .map(|token| format!("Cookie: {}={token}\r\n", sessions::COOKIE_NAME))
            .unwrap_or_default();
        let response = self.raw(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\n{origin}{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())).await;
        scan_logs();
        response
    }

    pub(crate) async fn raw(&self, request: String) -> Result<Response> {
        let address = self.address;
        let response = tokio::task::spawn_blocking(move || {
            let mut stream = std::net::TcpStream::connect(address)?;
            stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
            stream.write_all(request.as_bytes())?;
            let mut response = String::new();
            stream.read_to_string(&mut response)?;
            let (headers, body) = response.split_once("\r\n\r\n").ok_or("missing body")?;
            let status = headers
                .split_whitespace()
                .nth(1)
                .ok_or("missing status")?
                .parse()?;
            let headers = headers
                .lines()
                .skip(1)
                .filter_map(|line| {
                    line.split_once(':')
                        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
                })
                .collect();
            Ok(Response {
                status,
                headers,
                body: body.to_owned(),
            })
        })
        .await;
        scan_logs();
        response?
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}
impl Response {
    pub(crate) fn assert_error(&self, status: u16) {
        assert_eq!(self.status, status);
        assert!(self.body.is_empty());
        assert!(!self.headers.iter().any(|(key, _)| key == "set-cookie"));
    }
    pub(crate) fn cookie(&self) -> Result<String> {
        let header = self
            .headers
            .iter()
            .find(|(key, _)| key == "set-cookie")
            .ok_or("missing cookie")?;
        Ok(
            axum_extra::extract::cookie::Cookie::parse(header.1.clone())?
                .value()
                .to_owned(),
        )
    }
}

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = Capture(buffer.clone());
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .try_init()
            .unwrap();
        tracing::info!("session test log capture active");
        buffer
    })
}
/// Whether finding `value` in the logs would mean anything. The capture is shared by every
/// test in the binary and holds the harness's throwaway database names, which are random
/// hex; a short hex-only value turns up in them by chance, so it proves nothing (#56).
pub(crate) fn distinctive(value: &str) -> bool {
    value.len() >= 16 || !value.chars().all(|c| c.is_ascii_hexdigit())
}

fn check_distinctive(value: &str) {
    assert!(
        distinctive(value),
        "a short hex-only value can appear in unrelated log text; it is not a checkable secret"
    );
}

fn secrets() -> &'static Mutex<std::collections::BTreeSet<String>> {
    static SECRETS: OnceLock<Mutex<std::collections::BTreeSet<String>>> = OnceLock::new();
    SECRETS.get_or_init(Default::default)
}

pub(crate) fn register_secret(value: &str) {
    check_distinctive(value);
    logs();
    let escaped = format!("{value:?}");
    let mut secrets = secrets().lock().unwrap();
    secrets.insert(value.to_lowercase());
    secrets.insert(escaped[1..escaped.len() - 1].to_lowercase());
}

pub(crate) fn scan_logs() {
    // Release both locks before asserting: a caught failure must not poison later scans.
    let secrets = secrets().lock().unwrap().clone();
    let bytes = logs().lock().unwrap().clone();
    let logs = String::from_utf8_lossy(&bytes).to_lowercase();
    assert!(logs.contains("session test log capture active"));
    assert!(
        !secrets.iter().any(|secret| logs.contains(secret)),
        "secret appeared in captured logs"
    );
}

pub(crate) fn assert_no_secrets(secrets: &[&str]) {
    for secret in secrets {
        register_secret(secret);
    }
    scan_logs();
}

#[cfg(test)]
mod tests;
