use axum::{
    Router,
    http::Method,
    routing::{MethodFilter, MethodRouter, on},
};

use crate::{
    http, invites,
    sessions::{self, StateData},
};

pub(crate) const CREATE_PATH: &str = "/api/v1/organizations/{organization_id}/invites";
pub(crate) const REVOKE_PATH: &str = "/api/v1/organizations/{organization_id}/invites/{id}";
pub(crate) const ACCEPT_PATH: &str = "/api/v1/invite-acceptances";

pub(crate) struct Route {
    pub(crate) method: Method,
    pub(crate) path: &'static str,
    pub(crate) websocket: bool,
    pub(crate) needs_session: bool,
    handler: fn() -> MethodRouter<StateData>,
}

#[cfg(test)]
impl Route {
    pub(crate) fn changes_state(&self) -> bool {
        self.websocket
            || matches!(
                self.method,
                Method::POST | Method::PUT | Method::PATCH | Method::DELETE
            )
    }
}

// The method appears once so the sweep cannot test a different method from the router.
macro_rules! registry {
    ($(($method:ident, $path:expr, $websocket:literal, $session:literal, $handler:path)),* $(,)?) => {
        pub(crate) const ROUTES: &[Route] = &[$(Route {
            method: Method::$method,
            path: $path,
            websocket: $websocket,
            needs_session: $session,
            handler: || on(MethodFilter::$method, $handler),
        }),*];
    };
}

registry![
    (GET, "/health", false, false, http::health),
    (POST, "/api/v1/sessions", false, false, sessions::sign_in),
    (
        DELETE,
        "/api/v1/sessions/current",
        false,
        true,
        sessions::sign_out
    ),
    (GET, "/api/v1/me", false, true, sessions::me),
    (POST, CREATE_PATH, false, true, invites::create),
    (DELETE, REVOKE_PATH, false, true, invites::revoke),
    (POST, ACCEPT_PATH, false, false, invites::accept),
];

pub(crate) fn router() -> Router<StateData> {
    ROUTES.iter().fold(Router::new(), |router, route| {
        router.route(route.path, (route.handler)())
    })
}
