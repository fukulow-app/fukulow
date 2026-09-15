use std::env;

use anyhow::{Result, anyhow};
use tracing_subscriber::EnvFilter;

pub(crate) fn init() -> Result<()> {
    let (filter, invalid) = match env::var("RUST_LOG") {
        Ok(value) => match EnvFilter::try_new(value) {
            Ok(filter) => (filter, false),
            Err(_) => (EnvFilter::new("info"), true),
        },
        Err(env::VarError::NotPresent) => (EnvFilter::new("info"), false),
        Err(env::VarError::NotUnicode(_)) => (EnvFilter::new("info"), true),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init()
        .map_err(|_| anyhow!("Failed to initialise tracing subscriber"))?;
    if invalid {
        // Filter parse errors may contain arbitrary environment contents.
        tracing::warn!("Invalid RUST_LOG directive; falling back to info");
    }
    Ok(())
}
