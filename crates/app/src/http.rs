use std::{io::ErrorKind, net::SocketAddr};

use anyhow::{Context, Result, anyhow};
use axum::{Router, http::header, response::IntoResponse, routing::get};
use tokio::net::TcpListener;

pub(crate) fn routes() -> Router {
    Router::new().route("/health", get(health))
}

async fn health() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"status":"ok"}"#,
    )
}

pub(crate) async fn run(address: SocketAddr, routes: Router) -> Result<()> {
    // Register before announcing the listener so an immediate signal is handled.
    let shutdown = crate::shutdown::register()?;
    let listener = TcpListener::bind(address).await.map_err(|error| {
        if error.kind() == ErrorKind::AddrInUse {
            anyhow!("Failed to bind FUKULOW_BIND_ADDR: address is in use")
        } else {
            anyhow!(error).context("Failed to bind FUKULOW_BIND_ADDR")
        }
    })?;
    tracing::info!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, routes)
        .with_graceful_shutdown(shutdown)
        .await
        .context("HTTP listener failed")
}
