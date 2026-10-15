use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use futures_util::StreamExt;
use std::net::SocketAddr;

use crate::auth::extract_api_key;
use crate::error::GatewayError;
use crate::logs::RequestLogEntry;
use crate::openai::{ChatCompletionRequest, EmbeddingRequest};
use crate::AppState;

const MAX_REQUEST_BYTES: usize = 8 << 20; // 8 MiB

pub fn v1_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(handle_chat))
        .route("/v1/models", get(handle_models))
        .route("/v1/embeddings", post(handle_embeddings))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(state)
}

fn header_session_id(headers: &HeaderMap) -> String {
    headers
        .get("X-Session-ID")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

fn source_ip(headers: &HeaderMap, addr: &SocketAddr) -> String {
    if let Some(xff) = headers.get("X-Forwarded-For") {
        if let Ok(v) = xff.to_str() {
            if let Some(first) = v.split(',').next() {
                let ip = first.trim();
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        }
    }
    if let Some(real) = headers.get("X-Real-IP") {
        if let Ok(v) = real.to_str() {
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    addr.ip().to_string()
}

/// Heuristic background-task detection: clients mark background work via
/// X-Background-Task header or a `background` field; used by the router's
/// silent downgrade rule.
fn is_background_task(headers: &HeaderMap) -> bool {
    headers
        .get("X-Background-Task")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

async fn handle_chat(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let started = Instant::now();
    let api_key = extract_api_key(&headers);
    let src_ip = source_ip(&headers, &addr);

    let req: ChatCompletionRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return GatewayError::bad_request(format!("invalid json body: {}", e)).into_response()
        }
    };

    let mut session_id = header_session_id(&headers);
    if session_id.is_empty() {
        session_id = req.user.clone();
    }
    let client_model = req.model.clone();
    let is_stream = req.stream;

    // Auth + routing (model alias resolution, per-key model allowlist).
    let ctx = match state.gateway.authorize(&api_key, &client_model, is_background_task(&headers)) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    // Rewrite the request to the resolved upstream model, keeping the
    // client-visible model name for the response/logs.
    let mut upstream_req = req.clone();
    upstream_req.model = ctx.resolved.model.clone();
    if !ctx.resolved.effort.is_empty() && upstream_req.reasoning_effort.is_empty() {
        upstream_req.reasoning_effort = ctx.resolved.effort.clone();
    }

    let log_entry = |status: u16, started: Instant, ttft: u64, usage: Option<&crate::openai::Usage>| RequestLogEntry {
        ts: Utc::now(),
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        model: client_model.clone(),
        status,
        latency_ms: started.elapsed().as_millis() as u64,
        session_id: session_id.clone(),
        account_id: String::new(),
        ttft_ms: ttft,
        prompt_tokens: usage.map(|u| u.prompt_tokens).unwrap_or(0),
        completion_tokens: usage.map(|u| u.completion_tokens).unwrap_or(0),
        total_tokens: usage.map(|u| u.total_tokens).unwrap_or(0),
        source_ip: src_ip.clone(),
        key_name: ctx.key_name.clone(),
    };

    if is_stream {
        match state
            .gateway
            .chat_completion_stream(&upstream_req, &session_id)
            .await
        {
            Ok(stream) => {
                state.logs.record(log_entry(200, started, 0, None));
                let byte_stream = stream.map(|chunk| {
                    chunk.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                });
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/event-stream")
                    .header("Cache-Control", "no-cache")
                    .body(Body::from_stream(byte_stream))
                    .unwrap_or_else(|_| {
                        GatewayError::internal("failed to build stream response").into_response()
                    })
            }
            Err(e) => {
                let status = e.kind.status().as_u16();
                state.logs.record(log_entry(status, started, 0, None));
                e.into_response()
            }
        }
    } else {
        match state.gateway.chat_completion(&upstream_req).await {
            Ok(resp) => {
                if let Some(u) = &resp.usage {
                    state.gateway.record_usage(&ctx.key_hash, u.total_tokens);
                }
                state.logs.record(log_entry(200, started, 0, resp.usage.as_ref()));
                Json(resp).into_response()
            }
            Err(e) => {
                let status = e.kind.status().as_u16();
                state.logs.record(log_entry(status, started, 0, None));
                e.into_response()
            }
        }
    }
}

async fn handle_models(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let started = Instant::now();
    let api_key = extract_api_key(&headers);
    match state.gateway.models(&api_key).await {
        Ok(mut list) => {
            // Prepend user-defined aliases so clients can discover them.
            for (alias, _note) in state.router.advertised_aliases() {
                if !list.data.iter().any(|m| m.id == alias) {
                    list.data.insert(0, crate::openai::Model {
                        id: alias,
                        object: "model".to_string(),
                        owned_by: "codearts-gateway".to_string(),
                    });
                }
            }
            state.logs.record(RequestLogEntry {
                ts: Utc::now(),
                method: "GET".to_string(),
                path: "/v1/models".to_string(),
                model: String::new(),
                status: 200,
                latency_ms: started.elapsed().as_millis() as u64,
                session_id: String::new(),
                account_id: String::new(),
                ttft_ms: 0,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                source_ip: String::new(),
                key_name: String::new(),
            });
            Json(list).into_response()
        }
        Err(e) => e.into_response(),
    }
}

async fn handle_embeddings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let api_key = extract_api_key(&headers);
    let req: EmbeddingRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return GatewayError::bad_request(format!("invalid json body: {}", e)).into_response()
        }
    };
    match state.gateway.embeddings(&req, &api_key).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => e.into_response(),
    }
}
