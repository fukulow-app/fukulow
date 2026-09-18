use std::{io::ErrorKind, net::SocketAddr};

use anyhow::{Context, Result, anyhow};
use axum::{Router, http::header, response::IntoResponse, routing::get};
use tokio::net::TcpListener;

pub(crate) fn routes(state: crate::sessions::StateData) -> Router {
    Router::new()
        .route("/health", get(health))
        .merge(crate::sessions::routes())
        .layer(axum::middleware::from_fn_with_state(state.clone(), protect))
        .with_state(state)
}

async fn protect(
    axum::extract::State(state): axum::extract::State<crate::sessions::StateData>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{Method, StatusCode};
    let changes_state = matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    let websocket = request
        .headers()
        .get(header::UPGRADE)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"));
    if changes_state || websocket {
        let mut origins = request.headers().get_all(header::ORIGIN).iter();
        if origins
            .next()
            .is_none_or(|origin| origin.as_bytes() != state.origin.as_bytes())
            || origins.next().is_some()
        {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let response = next.run(request).await;
    // Extractor and router rejections share the same empty-error contract as handlers.
    if response.status().is_client_error() || response.status().is_server_error() {
        response.status().into_response()
    } else {
        response
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, http::StatusCode};
    use std::io::{Read, Write};

    #[tokio::test]
    async fn error_bodies_are_removed_for_every_status() -> Result<()> {
        let pool =
            sqlx::postgres::PgPoolOptions::new().connect_lazy(&std::env::var("DATABASE_URL")?)?;
        let state = crate::sessions::StateData::new(pool, "https://fixture.example.invalid".into());
        let router = Router::new()
            .route(
                "/{code}",
                get(|Path(code): Path<u16>| async move {
                    (
                        StatusCode::from_u16(code).unwrap(),
                        "fixture rejection details",
                    )
                }),
            )
            .layer(axum::middleware::from_fn_with_state(state, protect));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let result = tokio::task::spawn_blocking(move || -> Result<()> {
            for code in [401, 403, 404, 409, 422, 500] {
                let mut stream = std::net::TcpStream::connect(address)?;
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
                write!(
                    stream,
                    "GET /{code} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
                )?;
                let mut response = String::new();
                stream.read_to_string(&mut response)?;
                assert_eq!(
                    response.split_whitespace().nth(1),
                    Some(code.to_string().as_str())
                );
                assert!(response.split_once("\r\n\r\n").unwrap().1.is_empty());
            }
            Ok(())
        })
        .await;
        server.abort();
        result??;
        Ok(())
    }
}
