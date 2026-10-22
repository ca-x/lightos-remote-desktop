//! Opt-in MCP transport and authenticated browser management for the owned desktop.
mod store;
mod tools;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use serde::Deserialize;

pub use store::{Scope, TokenInfo, TokenStore};
use tools::{Desktop, DesktopTools};

use crate::{config::Config, server::origin_allowed, wayland::Backend};

pub(crate) fn settings_path(config: &Config) -> Option<PathBuf> {
    config.mcp_settings.clone().or_else(store::default_path)
}

pub(crate) fn management(store: Arc<TokenStore>) -> Router {
    Router::new()
        .route("/api/mcp", get(status).put(set_enabled))
        .route("/api/mcp/tokens", post(create_token))
        .route("/api/mcp/tokens/{id}", delete(revoke_token))
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn(management_origin))
        .with_state(store)
}

pub(crate) fn transport(
    store: Arc<TokenStore>,
    backend: Arc<Backend>,
    controller: Arc<std::sync::atomic::AtomicBool>,
    owned: bool,
) -> Router {
    transport_with_desktop(store, Desktop::new(backend, controller, owned))
}

fn transport_with_desktop(store: Arc<TokenStore>, desktop: Desktop) -> Router {
    let tool_store = store.clone();
    let mut config = StreamableHttpServerConfig::default();
    config.legacy_session_mode = false;
    config.json_response = true;
    config.max_request_body_bytes = 64 * 1024;
    // Bearer authentication and same-origin checks guard arbitrary proxy hosts.
    config.allowed_hosts.clear();
    let service = StreamableHttpService::new(
        move || Ok(DesktopTools::new(tool_store.clone(), desktop.clone())),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(store, authenticate))
}

async fn authenticate(
    State(store): State<Arc<TokenStore>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if request.headers().contains_key(header::ORIGIN) && !same_origin(request.headers())
        || request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|value| value == "cross-site")
    {
        return (StatusCode::FORBIDDEN, "Same-origin MCP requests required").into_response();
    }
    if !store.enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "MCP is disabled or unavailable",
        )
            .into_response();
    }
    let principal = (request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .count()
        == 1)
        .then(|| request.headers().get(header::AUTHORIZATION))
        .flatten()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .and_then(|value| store.authenticate(value));
    let Some(principal) = principal else {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"LightOS MCP\"")],
            "MCP bearer token required",
        )
            .into_response();
    };
    // rmcp forwards HTTP parts to tools; keep the plaintext credential out of
    // protocol extensions and any downstream Debug/trace instrumentation.
    request.headers_mut().remove(header::AUTHORIZATION);
    request.extensions_mut().insert(principal);
    next.run(request).await
}

fn same_origin(headers: &HeaderMap) -> bool {
    headers.get_all(header::ORIGIN).iter().count() == 1
        && headers.get_all(header::HOST).iter().count() == 1
        && origin_allowed(headers)
}

async fn management_origin(request: Request<Body>, next: Next) -> Response {
    let headers = request.headers();
    // Same-origin GET fetches omit Origin. Fetch Metadata plus a custom header
    // permits those requests while forcing foreign browsers through preflight.
    let same_origin = if headers.contains_key(header::ORIGIN) {
        same_origin(headers)
    } else {
        request.method() == axum::http::Method::GET
            && headers
                .get("sec-fetch-site")
                .is_some_and(|value| value == "same-origin")
    };
    if !same_origin
        || headers
            .get("x-lightos-mcp")
            .is_none_or(|value| value != "1")
    {
        return (StatusCode::FORBIDDEN, "Same-origin MCP management required").into_response();
    }
    next.run(request).await
}

fn failure(error: anyhow::Error) -> Response {
    // Never serialize arbitrary file contents or credentials from an error.
    tracing::warn!(%error, "MCP management failed");
    (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error":"MCP settings unavailable; check the private settings file and its permissions."}))).into_response()
}

async fn status(State(store): State<Arc<TokenStore>>) -> Response {
    match store.status() {
        Ok(status) => Json(status).into_response(),
        Err(error) => failure(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnableRequest {
    enabled: bool,
}

async fn set_enabled(
    State(store): State<Arc<TokenStore>>,
    Json(request): Json<EnableRequest>,
) -> Response {
    match tokio::task::spawn_blocking(move || store.set_enabled(request.enabled)).await {
        Ok(Ok(status)) => Json(status).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "MCP settings unavailable").into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    name: String,
    scope: Scope,
}

async fn create_token(
    State(store): State<Arc<TokenStore>>,
    Json(request): Json<CreateRequest>,
) -> Response {
    if request.name.trim().is_empty()
        || request.name.chars().count() > 80
        || request.name.chars().any(char::is_control)
    {
        return (
            StatusCode::BAD_REQUEST,
            "Token name must be 1–80 characters without control characters",
        )
            .into_response();
    }
    if store.status().is_ok_and(|status| status.tokens.len() >= 32) {
        return (
            StatusCode::CONFLICT,
            "Revoke a token before creating more (maximum 32)",
        )
            .into_response();
    }
    match tokio::task::spawn_blocking(move || store.create(&request.name, request.scope)).await {
        Ok(Ok(token)) => (StatusCode::CREATED, Json(token)).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "MCP settings unavailable").into_response(),
    }
}

async fn revoke_token(State(store): State<Arc<TokenStore>>, Path(id): Path<String>) -> Response {
    if id.len() != 22
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::task::spawn_blocking(move || store.revoke(&id)).await {
        Ok(Ok(true)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Ok(false)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "MCP settings unavailable").into_response(),
    }
}

#[cfg(test)]
mod tests;
