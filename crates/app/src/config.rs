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
    if env::var_os("INSPECTOR_DATABASE_URL").is_some() {
        return Err(anyhow!(
            "INSPECTOR_DATABASE_URL must not be set for the application"
        ));
    }
    env::var("DATABASE_URL")
        .map_err(|_| anyhow!("DATABASE_URL is required and must be valid Unicode"))
}

pub(crate) fn public_origin() -> Result<String> {
    let value = env::var("FUKULOW_PUBLIC_ORIGIN")
        .map_err(|_| anyhow!("FUKULOW_PUBLIC_ORIGIN is required and must be valid Unicode"))?;
    validate_public_origin(&value)?;
    Ok(value)
}

pub(crate) fn validate_public_origin(value: &str) -> Result<()> {
    let invalid = || {
        anyhow!(
            "FUKULOW_PUBLIC_ORIGIN must contain only scheme, host and optional port, without credentials, path, query, fragment or trailing slash"
        )
    };
    let (scheme, authority) = value.split_once("://").ok_or_else(invalid)?;
    if authority.is_empty() || authority.contains(['/', '?', '#', '@']) {
        return Err(invalid());
    }
    let uri: axum::http::Uri = value.parse().map_err(|_| invalid())?;
    let host = uri.host().ok_or_else(invalid)?;
    let parsed_authority = uri.authority().ok_or_else(invalid)?;
    if parsed_authority.as_str() != authority || uri.scheme_str() != Some(scheme) {
        return Err(invalid());
    }
    let suffix = authority.strip_prefix(host).ok_or_else(invalid)?;
    if !suffix.is_empty() && (!suffix.starts_with(':') || suffix[1..].parse::<u16>().is_err()) {
        return Err(invalid());
    }
    if scheme != "https"
        && !(scheme == "http" && matches!(host, "localhost" | "127.0.0.1" | "[::1]"))
    {
        return Err(anyhow!(
            "FUKULOW_PUBLIC_ORIGIN requires HTTPS because the Secure __Host- session cookie is refused on insecure origins; HTTP is allowed only on localhost, 127.0.0.1 or [::1] for development"
        ));
    }
    Ok(())
}
