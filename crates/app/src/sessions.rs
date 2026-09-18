use axum::{
    Json,
    body::Bytes,
    extract::{FromRequestParts, State, rejection::BytesRejection},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use domain::ActorId;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

pub(crate) const COOKIE_NAME: &str = "__Host-fukulow_session";
const SESSION_LIFETIME: Duration = Duration::days(14);

type TokenGenerator = fn() -> Result<(auth::SessionToken, db::TokenHash), auth::TokenError>;

#[derive(Clone)]
pub(crate) struct StateData {
    pub(crate) pool: PgPool,
    pub(crate) origin: String,
    pub(crate) new_token: TokenGenerator,
    pub(crate) now: fn() -> OffsetDateTime,
    pub(crate) new_invite_token: crate::invites::TokenGenerator,
    pub(crate) hash_invite_password: std::sync::Arc<crate::invites::PasswordHasher>,
}

impl StateData {
    pub(crate) fn new(pool: PgPool, origin: String) -> Self {
        Self {
            pool,
            origin,
            new_token: auth::new_session_token,
            new_invite_token: auth::new_invite_token,
            hash_invite_password: std::sync::Arc::new(auth::hash_password),
            now: OffsetDateTime::now_utc,
        }
    }
}

/// Only sign-out and the future WebSocket handler may consume this lookup key.
pub(crate) struct CurrentSession(db::TokenHash);

pub(crate) struct Authenticated {
    pub(crate) actor: ActorId,
    session: CurrentSession,
}

impl FromRequestParts<StateData> for Authenticated {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &StateData,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar.get(COOKIE_NAME).ok_or(StatusCode::UNAUTHORIZED)?;
        let hash = auth::hash_session_token(token.value());
        let actor = db::resolve_session(&state.pool, &hash)
            .await
            .map_err(session_error)?;
        Ok(Self {
            actor,
            session: CurrentSession(hash),
        })
    }
}

pub(crate) async fn sign_in(
    State(state): State<StateData>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, StatusCode> {
    let body = body.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let value: Value =
        serde_json::from_slice(&body).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let object = value.as_object().ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    let email = object
        .get("email")
        .and_then(Value::as_str)
        // PostgreSQL text cannot hold U+0000, so such an address cannot be looked up; it
        // is refused here as a malformed body rather than reaching the database as a 500.
        .filter(|email| !email.contains('\0'))
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    let password = object
        .get("password")
        .and_then(Value::as_str)
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?
        .to_owned();
    let new_token = state.new_token;
    let (_, token) = db::sign_in(
        &state.pool,
        email,
        move |phc| auth::verify_password(&password, phc),
        move || new_token().ok(),
        (state.now)() + SESSION_LIFETIME,
    )
    .await
    .map_err(|error| match error {
        db::SignInError::InvalidCredentials => StatusCode::UNAUTHORIZED,
        db::SignInError::TokenUnavailable | db::SignInError::Database(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    })?;
    let jar = CookieJar::new().add(cookie(token.as_str().to_owned(), SESSION_LIFETIME));
    Ok((jar, StatusCode::NO_CONTENT).into_response())
}

pub(crate) async fn sign_out(
    State(state): State<StateData>,
    authenticated: Authenticated,
) -> Result<Response, StatusCode> {
    db::revoke_session(&state.pool, authenticated.actor, &authenticated.session.0)
        .await
        .map_err(session_error)?;
    let jar = CookieJar::new().add(cookie(String::new(), Duration::ZERO));
    Ok((jar, StatusCode::NO_CONTENT).into_response())
}

pub(crate) async fn me(
    State(state): State<StateData>,
    authenticated: Authenticated,
) -> Result<Json<Value>, StatusCode> {
    let me = db::me(&state.pool, authenticated.actor)
        .await
        .map_err(session_error)?;
    Ok(Json(me_json(me)))
}

fn cookie(value: String, lifetime: Duration) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, value))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(lifetime)
        .build()
}

pub(crate) fn me_json(me: db::Me) -> Value {
    let mut value = json!({ "actor": {
        "id": me.actor_id.0.to_string(), "type": me.actor_type.as_str(), "display_name": me.display_name
    }});
    if let Some(email) = me.email {
        value["user"] = json!({"email": email});
    }
    value
}

fn session_error(error: db::SessionError) -> StatusCode {
    match error {
        db::SessionError::NoSession => StatusCode::UNAUTHORIZED,
        db::SessionError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
