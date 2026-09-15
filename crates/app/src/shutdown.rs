use std::future::Future;

use anyhow::{Context, Result};
// Containers send SIGTERM, which ctrl_c alone would miss; Windows is not
// a supported host yet.
use tokio::signal::unix::{SignalKind, signal};

pub(crate) fn register() -> Result<impl Future<Output = ()> + Send + 'static> {
    let mut interrupt = signal(SignalKind::interrupt()).context("Failed to register SIGINT")?;
    let mut terminate = signal(SignalKind::terminate()).context("Failed to register SIGTERM")?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {},
            _ = terminate.recv() => {},
        }
        tracing::info!("Shutdown signal received; draining in-flight requests");
    })
}
