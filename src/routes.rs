use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use futures_util::StreamExt;

use crate::auth::extract_api_key;
use crate::error::GatewayError;
use crate::logs::RequestLogEntry;
use crate::openai::{ChatCompletionRequest, EmbeddingRequest};
use crate::AppState;

const MAX_REQUEST_BYTES: usize = 8 << 20; // 8 MiB, same as the legacy gateway

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

async fn handle_chat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let started = Instant::now();
    let api_key = extract_api_key(&headers);

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
    let model = req.model.clone();

    if req.stream {
        match state
            .gateway
            .chat_completion_stream(&req, &session_id, &api_key)
            .await
        {
            Ok(stream) => {
                state.logs.record(RequestLogEntry {
                    ts: Utc::now(),
                    method: "POST".to_string(),
                    path: "/v1/chat/completions".to_string(),
                    model,
                    status: 200,
                    latency_ms: started.elapsed().as_millis() as u64,
                    session_id,
                    account_id: String::new(),
                });
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
                state.logs.record(RequestLogEntry {
                    ts: Utc::now(),
                    method: "POST".to_string(),
                    path: "/v1/chat/completions".to_string(),
                    model,
                    status,
                    latency_ms: started.elapsed().as_millis() as u64,
                    session_id,
                    account_id: String::new(),
                });
                e.into_response()
            }
        }
    } else {
        match state.gateway.chat_completion(&req, &api_key).await {
            Ok(resp) => {
                state.logs.record(RequestLogEntry {
                    ts: Utc::now(),
                    method: "POST".to_string(),
                    path: "/v1/chat/completions".to_string(),
                    model,
                    status: 200,
                    latency_ms: started.elapsed().as_millis() as u64,
                    session_id,
                    account_id: String::new(),
                });
                Json(resp).into_response()
            }
            Err(e) => {
                let status = e.kind.status().as_u16();
                state.logs.record(RequestLogEntry {
                    ts: Utc::now(),
                    method: "POST".to_string(),
                    path: "/v1/chat/completions".to_string(),
                    model,
                    status,
                    latency_ms: started.elapsed().as_millis() as u64,
                    session_id,
                    account_id: String::new(),
                });
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
        Ok(list) => {
            state.logs.record(RequestLogEntry {
                ts: Utc::now(),
                method: "GET".to_string(),
                path: "/v1/models".to_string(),
                model: String::new(),
                status: 200,
                latency_ms: started.elapsed().as_millis() as u64,
                session_id: String::new(),
                account_id: String::new(),
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
