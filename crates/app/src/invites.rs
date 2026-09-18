use crate::sessions::{Authenticated, StateData};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{State, rejection::BytesRejection},
    http::{StatusCode, Uri},
    routing::{delete, post},
};
use domain::{Capability, InviteId, OrganizationId};
use serde_json::{Map, Value, json};
use sqlx::types::Uuid;
use time::{Duration, UtcOffset, format_description::well_known::Rfc3339};

pub(crate) type TokenGenerator =
    fn() -> Result<(auth::InviteToken, db::TokenHash), auth::TokenError>;
pub(crate) type PasswordHasher =
    dyn Fn(&str) -> Result<String, auth::PasswordHashError> + Send + Sync;

pub(crate) const CREATE_PATH: &str = "/api/v1/organizations/{organization_id}/invites";
pub(crate) const REVOKE_PATH: &str = "/api/v1/organizations/{organization_id}/invites/{id}";
pub(crate) const ACCEPT_PATH: &str = "/api/v1/invite-acceptances";

pub(crate) fn routes() -> Router<StateData> {
    Router::new()
        .route(CREATE_PATH, post(create))
        .route(REVOKE_PATH, delete(revoke))
        .route(ACCEPT_PATH, post(accept))
}

async fn create(
    State(state): State<StateData>,
    authenticated: Authenticated,
    uri: Uri,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let fields = object(body)?;
    let expires_in = integer(&fields, "expires_in", 300, 604_800)?;
    let max_uses = if fields.contains_key("max_uses") {
        integer(&fields, "max_uses", 1, 100)? as i32
    } else {
        1
    };
    let organization = OrganizationId(path_identifier(&uri, 4)?);
    authorize(&state, authenticated.actor, organization).await?;
    let (token, hash) =
        (state.new_invite_token)().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let expires_at = ((state.now)() + Duration::seconds(expires_in)).to_offset(UtcOffset::UTC);
    let formatted = expires_at
        .format(&Rfc3339)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let id = db::create_invite(
        &state.pool,
        authenticated.actor,
        organization,
        &hash,
        expires_at,
        max_uses,
    )
    .await
    .map_err(|error| match error {
        db::CreateInviteError::NotFound => StatusCode::NOT_FOUND,
        db::CreateInviteError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    })?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id.0.to_string(), "token": token.as_str(),
        "link": format!("{}/invite#{}", state.origin, token.as_str()),
        "expires_at": formatted, "max_uses": max_uses})),
    ))
}

async fn revoke(
    State(state): State<StateData>,
    authenticated: Authenticated,
    uri: Uri,
) -> Result<StatusCode, StatusCode> {
    let organization = OrganizationId(path_identifier(&uri, 4)?);
    authorize(&state, authenticated.actor, organization).await?;
    let invite = InviteId(path_identifier(&uri, 6)?);
    db::revoke_invite(&state.pool, authenticated.actor, organization, invite)
        .await
        .map_err(|error| match error {
            db::RevokeInviteError::NotFound => StatusCode::NOT_FOUND,
            db::RevokeInviteError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn accept(
    State(state): State<StateData>,
    body: Result<Bytes, BytesRejection>,
) -> Result<StatusCode, StatusCode> {
    let fields = object(body)?;
    let token = string(&fields, "token")?;
    let hash = auth::hash_invite_token(token);
    if !db::invite_is_usable(&state.pool, &hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let (email, password, display_name) = acceptance_fields(&fields)?;
    let password = password.to_owned();
    // A usable-token filter precedes argon2, which must not hold the shared invite's row lock.
    let hasher = state.hash_invite_password;
    let password_hash = tokio::task::spawn_blocking(move || hasher(&password))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    db::accept_invite(&state.pool, &hash, email, &password_hash, display_name)
        .await
        .map_err(|error| match error {
            db::AcceptInviteError::InviteUnusable => StatusCode::NOT_FOUND,
            db::AcceptInviteError::EmailTaken => StatusCode::CONFLICT,
            db::AcceptInviteError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        })?;
    Ok(StatusCode::NO_CONTENT)
}

fn object(body: Result<Bytes, BytesRejection>) -> Result<Map<String, Value>, StatusCode> {
    let body = body.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    match serde_json::from_slice(&body) {
        Ok(Value::Object(fields)) => Ok(fields),
        _ => Err(StatusCode::UNPROCESSABLE_ENTITY),
    }
}

fn integer(fields: &Map<String, Value>, key: &str, min: i64, max: i64) -> Result<i64, StatusCode> {
    fields
        .get(key)
        .and_then(Value::as_i64)
        .filter(|value| (min..=max).contains(value))
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)
}

fn string<'a>(fields: &'a Map<String, Value>, key: &str) -> Result<&'a str, StatusCode> {
    fields
        .get(key)
        .and_then(Value::as_str)
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)
}

fn acceptance_fields(fields: &Map<String, Value>) -> Result<(&str, &str, &str), StatusCode> {
    let email = string(fields, "email")?;
    let password = string(fields, "password")?;
    let display_name = string(fields, "display_name")?.trim();
    if !(3..=254).contains(&email.len())
        || email.chars().filter(|&c| c == '@').count() != 1
        || email.chars().any(|c| c.is_whitespace() || c == '\0')
        || password.chars().count() < 8
        || password.len() > 1024
        || !(1..=80).contains(&display_name.chars().count())
        || display_name.contains('\0')
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    Ok((email, password, display_name))
}

// Path rejects invalid UTF-8 before the body or capability check can decide the response.
fn path_identifier(uri: &Uri, segment: usize) -> Result<Uuid, StatusCode> {
    let raw = uri
        .path()
        .split('/')
        .nth(segment)
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut bytes = raw.bytes();
    let mut decoded = Vec::with_capacity(36);
    while let Some(byte) = bytes.next() {
        let byte = if byte == b'%' {
            let high = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or(StatusCode::NOT_FOUND)?;
            let low = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or(StatusCode::NOT_FOUND)?;
            (high * 16 + low) as u8
        } else {
            byte
        };
        decoded.push(byte);
        if decoded.len() > 36 {
            return Err(StatusCode::NOT_FOUND);
        }
    }
    let value = std::str::from_utf8(&decoded).map_err(|_| StatusCode::NOT_FOUND)?;
    identifier(value)
}

fn identifier(value: &str) -> Result<Uuid, StatusCode> {
    if value.len() != 36
        || ![8, 13, 18, 23]
            .into_iter()
            .all(|i| value.as_bytes()[i] == b'-')
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Uuid::parse_str(value).map_err(|_| StatusCode::NOT_FOUND)
}

async fn authorize(
    state: &StateData,
    actor: domain::ActorId,
    organization: OrganizationId,
) -> Result<(), StatusCode> {
    match db::can(&state.pool, actor, organization, Capability::InviteMember)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        db::Access::Allowed => Ok(()),
        db::Access::Forbidden => Err(StatusCode::FORBIDDEN),
        db::Access::NotAMember => Err(StatusCode::NOT_FOUND),
    }
}
