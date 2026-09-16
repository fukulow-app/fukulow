use std::{env, net::SocketAddr};

use anyhow::{Result, anyhow};

pub(crate) fn bind_address() -> Result<SocketAddr> {
    match env::var("FUKULOW_BIND_ADDR") {
        Ok(value) => value
            .parse()
            .map_err(|_| anyhow!("FUKULOW_BIND_ADDR must be a valid socket address")),
        Err(env::VarError::NotPresent) => Ok(SocketAddr::from(([127, 0, 0, 1], 8080))),
        // VarError can contain the rejected value, so it must not become a source.
        Err(env::VarError::NotUnicode(_)) => {
            Err(anyhow!("FUKULOW_BIND_ADDR must be a valid socket address"))
        }
    }
}

pub(crate) fn database_url() -> Result<String> {
    env::var("DATABASE_URL")
        .map_err(|_| anyhow!("DATABASE_URL is required and must be valid Unicode"))
}
