//! SDK sessions are isolated by authenticated token, with HTTP disconnect cancellation.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::any,
};
use futures_util::StreamExt;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, ProtocolVersion};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService,
    session::{SessionManager, local::LocalSessionManager},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use super::{
    store::{Principal, Scope, TokenStore},
    tools::{Desktop, DesktopTools},
};

#[derive(Clone)]
pub(super) struct HttpCancellation(pub CancellationToken);

type Service = StreamableHttpService<DesktopTools, LocalSessionManager>;
struct TokenService {
    service: Service,
    sessions: Arc<LocalSessionManager>,
    cancellation: CancellationToken,
    initialize: tokio::sync::Mutex<()>,
}
#[derive(Default)]
struct TokenServices(Mutex<HashMap<String, Arc<TokenService>>>);
impl Drop for TokenServices {
    fn drop(&mut self) {
        for entry in self.0.get_mut().unwrap().values() {
            entry.cancellation.cancel();
        }
    }
}
#[derive(Clone)]
struct Transport {
    store: Arc<TokenStore>,
    desktop: Desktop,
    tokens: Arc<TokenServices>,
}
impl Transport {
    fn service(&self, principal: &Principal) -> Arc<TokenService> {
        let mut tokens = self.tokens.0.lock().unwrap();
        tokens.retain(|_, entry| !entry.cancellation.is_cancelled());
        tokens
            .entry(principal.id.clone())
            .or_insert_with(|| {
                let mut sessions = LocalSessionManager::default();
                sessions.session_config.init_timeout = Some(Duration::from_secs(5));
                sessions.session_config.sse_retry = None;
                sessions.session_config.completed_cache_ttl = Duration::from_secs(1);
                let sessions = Arc::new(sessions);
                let cancellation = principal.cancellation.child_token();
                let mut config = StreamableHttpServerConfig::default();
                config.legacy_session_mode = true;
                config.sse_retry = None;
                config.max_request_body_bytes = 64 * 1024;
                config.allowed_hosts.clear();
                config.cancellation_token = cancellation.clone();
                let store = self.store.clone();
                let desktop = self.desktop.clone();
                let service = StreamableHttpService::new(
                    move || Ok(DesktopTools::new(store.clone(), desktop.clone())),
                    sessions.clone(),
                    config,
                );
                let stop = cancellation.clone();
                let cleanup = sessions.clone();
                tokio::spawn(async move {
                    stop.cancelled().await;
                    let ids: Vec<_> = cleanup.sessions.read().await.keys().cloned().collect();
                    for id in ids {
                        let _ = cleanup.close_session(&id).await;
                    }
                });
                Arc::new(TokenService {
                    service,
                    sessions,
                    cancellation,
                    initialize: tokio::sync::Mutex::new(()),
                })
            })
            .clone()
    }
}

pub(super) fn router(store: Arc<TokenStore>, desktop: Desktop) -> Router {
    Router::new()
        .route("/mcp", any(dispatch))
        .with_state(Transport {
            store: store.clone(),
            desktop,
            tokens: Arc::new(TokenServices::default()),
        })
        .layer(middleware::from_fn_with_state(store, super::authenticate))
}

async fn dispatch(State(transport): State<Transport>, mut request: Request<Body>) -> Response {
    // rmcp selects stateless/session routing before its initialize handler
    // negotiates versions. Normalize a modern initialize offer to our highest
    // supported session revision, using the SDK model for decoding/encoding.
    if request.method() == axum::http::Method::POST
        && !request.headers().contains_key("mcp-session-id")
    {
        let (mut parts, body) = request.into_parts();
        let bytes = match axum::body::to_bytes(body, 64 * 1024).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return (StatusCode::PAYLOAD_TOO_LARGE, "MCP body exceeds 64 KiB").into_response();
            }
        };
        let mut message = serde_json::from_slice::<ClientJsonRpcMessage>(&bytes).ok();
        let mut normalized = false;
        if let Some(ClientJsonRpcMessage::Request(ref mut rpc)) = message
            && let ClientRequest::InitializeRequest(ref mut init) = rpc.request
            && init.params.protocol_version.as_str() >= "2026-07-28"
        {
            if parts
                .headers
                .get("mcp-protocol-version")
                .is_some_and(|v| v.as_bytes() != init.params.protocol_version.as_str().as_bytes())
            {
                return (
                    StatusCode::BAD_REQUEST,
                    "MCP protocol header must match initialize offer",
                )
                    .into_response();
            }
            init.params.protocol_version = ProtocolVersion::V_2025_11_25;
            if parts.headers.contains_key("mcp-protocol-version") {
                parts
                    .headers
                    .insert("mcp-protocol-version", "2025-11-25".parse().unwrap());
            }
            parts.headers.remove(axum::http::header::CONTENT_LENGTH);
            normalized = true;
        }
        request = Request::from_parts(
            parts,
            if normalized {
                Body::from(serde_json::to_vec(&message.unwrap()).expect("SDK request serializes"))
            } else {
                Body::from(bytes)
            },
        );
    }
    if request
        .headers()
        .get("mcp-protocol-version")
        .is_some_and(|v| v.as_bytes() >= b"2026-07-28".as_slice())
    {
        return (
            StatusCode::BAD_REQUEST,
            "Use the session protocol returned by initialize",
        )
            .into_response();
    }
    if request.headers().get_all("mcp-session-id").iter().count() > 1 {
        return (StatusCode::BAD_REQUEST, "One MCP session ID required").into_response();
    }
    let principal = request
        .extensions()
        .get::<Principal>()
        .expect("authenticated MCP route");
    // Body collection can outlive a disable/revoke; do not route stale principals.
    if !transport.store.authorized(principal, Scope::Observe) {
        return (StatusCode::UNAUTHORIZED, "MCP token disabled or revoked").into_response();
    }
    let entry = transport.service(principal);
    if let Some(id) = request.headers().get("mcp-session-id") {
        let Ok(id) = id.to_str() else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        if !entry
            .sessions
            .has_session(&id.to_owned().into())
            .await
            .unwrap_or(false)
        {
            return (
                StatusCode::NOT_FOUND,
                "MCP session not found for this token",
            )
                .into_response();
        }
    }
    let _initialization = if request.method() == axum::http::Method::POST
        && !request.headers().contains_key("mcp-session-id")
    {
        let lock = entry.initialize.lock().await;
        if entry.sessions.sessions.read().await.len() >= 8 {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Maximum eight MCP sessions per token; close an existing session",
            )
                .into_response();
        }
        Some(lock)
    } else {
        None
    };
    let cancellation = CancellationToken::new();
    let guard = cancellation.clone().drop_guard();
    request
        .extensions_mut()
        .insert(HttpCancellation(cancellation));
    let response = match entry.service.clone().oneshot(request).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    let (parts, body) = response.into_parts();
    let stream = futures_util::stream::unfold(
        (Body::new(body).into_data_stream(), guard),
        |(mut body, guard)| async move { body.next().await.map(|chunk| (chunk, (body, guard))) },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}
