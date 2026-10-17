use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::{Body, Bytes},
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
use crate::openai::{ChatCompletionRequest, EmbeddingRequest, Usage};
use crate::sse_tap::SseTap;
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

/// B4: source IP extraction. Proxy headers are only trusted when
/// --trust-proxy-headers is enabled (default); otherwise the connection
/// address is authoritative.
fn source_ip(headers: &HeaderMap, addr: &SocketAddr, trust_proxy: bool) -> String {
    if trust_proxy {
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
    }
    addr.ip().to_string()
}

/// Writes the streaming audit entry exactly once when the response body
/// is dropped (normal completion or client disconnect alike).
struct StreamFinalize {
    tap: std::sync::Arc<std::sync::Mutex<SseTap>>,
    state: Arc<AppState>,
    key_hash: String,
    model: String,
    session_id: String,
    source_ip: String,
    key_name: String,
    /// B4: account attribution and resolution metadata.
    account_id: String,
    attempts: u32,
    retry_account_ids: Vec<String>,
    route_via: String,
    resolved_model: String,
    downgrade_observed: bool,
}

impl Drop for StreamFinalize {
    fn drop(&mut self) {
        let audit = match self.tap.lock() {
            Ok(t) => t.clone_finish(),
            Err(_) => return,
        };
        if audit.total_tokens > 0 {
            self.state.gateway.record_usage(&self.key_hash, audit.total_tokens);
        }
        self.state.logs.record(RequestLogEntry {
            ts: Utc::now(),
            method: "POST".to_string(),
            path: "/v1/chat/completions".to_string(),
            model: self.model.clone(),
            status: 200,
            latency_ms: audit.total_ms,
            session_id: self.session_id.clone(),
            account_id: self.account_id.clone(),
            ttft_ms: audit.ttft_ms,
            prompt_tokens: audit.prompt_tokens,
            completion_tokens: audit.completion_tokens,
            total_tokens: audit.total_tokens,
            credit: audit.credit,
            cached_tokens: audit.cached_tokens,
            source_ip: self.source_ip.clone(),
            key_name: self.key_name.clone(),
            route_via: self.route_via.clone(),
            resolved_model: self.resolved_model.clone(),
            attempts: self.attempts,
            retry_account_ids: self.retry_account_ids.clone(),
            downgrade_observed: self.downgrade_observed,
        });
    }
}

/// B2: rewrite the `model` field inside an SSE chunk from the resolved
/// upstream name back to the client-supplied name (silent downgrade).
fn rewrite_stream_model(bytes: &Bytes, resolved: &str, client: &str) -> Bytes {
    if resolved == client || resolved.is_empty() {
        return bytes.clone();
    }
    let from = format!("\"model\":\"{}\"", resolved);
    let to = format!("\"model\":\"{}\"", client);
    let text = String::from_utf8_lossy(bytes);
    if text.contains(&from) {
        Bytes::from(text.replace(&from, &to))
    } else {
        bytes.clone()
    }
}

/// B2 multi-signal background detection, first decidable signal wins:
/// 1. `X-Background-Task` header: 1/true -> background, 0/false -> foreground.
/// 2. Request body `background` boolean field (gateway extension).
/// 3. Heuristic (only when the downgrade rule enables it): tools + tool-role
///    history, `bg-` user prefix; at least 2 signals required.
fn detect_background(headers: &HeaderMap, req: &ChatCompletionRequest, heuristic_enabled: bool) -> bool {
    if let Some(v) = headers.get("X-Background-Task").and_then(|v| v.to_str().ok()) {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            return true;
        }
        if v == "0" || v.eq_ignore_ascii_case("false") {
            return false;
        }
    }
    if let Some(bg) = req.background {
        return bg;
    }
    if !heuristic_enabled {
        return false;
    }
    let has_tools = req.tools.as_ref().map(|t| !t.is_empty()).unwrap_or(false);
    let has_tool_history = req.messages.iter().any(|m| m.role == "tool");
    let bg_user = req.user.starts_with("bg-");
    let signals = [has_tools && has_tool_history, bg_user]
        .iter()
        .filter(|s| **s)
        .count();
    signals >= 2
}

async fn handle_chat(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let started = Instant::now();
    let api_key = extract_api_key(&headers);
    let src_ip = source_ip(&headers, &addr, state.trust_proxy_headers);

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
    // B2: heuristic flag comes from the downgrade rule config.
    let heuristic = state
        .router
        .config()
        .downgrade
        .as_ref()
        .map(|d| d.use_heuristic)
        .unwrap_or(false);
    let ctx = match state
        .gateway
        .authorize(&api_key, &client_model, detect_background(&headers, &req, heuristic))
    {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    // A2: 若本次鉴权触发了旧摘要的惰性升级，立即把新 argon2id 摘要
    // 落盘 keys.json；落盘失败仅记 ERROR，下次请求重试升级。
    if !state.keys.take_dirty().is_empty() {
        if let Err(e) = state.key_store.save_keys(&state.keys.list()) {
            tracing::error!("failed to persist upgraded key hashes: {}", e);
        }
    }

    // Rewrite the request to the resolved upstream model, keeping the
    // client-visible model name for the response/logs.
    let mut upstream_req = req.clone();
    upstream_req.model = ctx.resolved.model.clone();
    if !ctx.resolved.effort.is_empty() && upstream_req.reasoning_effort.is_empty() {
        upstream_req.reasoning_effort = ctx.resolved.effort.clone();
    }

    let log_entry = |status: u16,
                     started: Instant,
                     ttft: u64,
                     usage: Option<&Usage>,
                     credit: Option<f64>,
                     cached: Option<u64>| RequestLogEntry {
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
        credit,
        cached_tokens: cached,
        source_ip: src_ip.clone(),
        key_name: ctx.key_name.clone(),
        route_via: ctx.resolved.via.as_str().to_string(),
        resolved_model: ctx.resolved.model.clone(),
        attempts: 1,
        retry_account_ids: Vec::new(),
        downgrade_observed: ctx.resolved.downgrade_observed,
    };

    if is_stream {
        match state
            .gateway
            .chat_completion_stream(&upstream_req, &session_id)
            .await
        {
            Ok((stream, meta)) => {
                // Side-channel tap: bytes flow downstream untouched while
                // we collect ttft + final usage (tokens, credit, cache hits).
                // When the stream ends (or the client disconnects), the tap
                // is dropped and the audit entry is written from Drop.
                let tap = std::sync::Arc::new(std::sync::Mutex::new(SseTap::new(started)));
                let tap_for_items = tap.clone();
                // B2: 静默降级——SSE 帧中的 model 字段改写为客户端传入名。
                let resolved_model = ctx.resolved.model.clone();
                let client_model_for_stream = client_model.clone();
                let audited = stream.map(move |item| {
                    match item {
                        Ok(bytes) => {
                            if let Ok(mut t) = tap_for_items.lock() {
                                t.observe(&bytes);
                            }
                            let rewritten = rewrite_stream_model(&bytes, &resolved_model, &client_model_for_stream);
                            Ok::<_, std::io::Error>(rewritten)
                        }
                        Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
                    }
                });
                let finalize = StreamFinalize {
                    tap,
                    state: state.clone(),
                    key_hash: ctx.key_hash.clone(),
                    model: client_model.clone(),
                    session_id: session_id.clone(),
                    source_ip: src_ip.clone(),
                    key_name: ctx.key_name.clone(),
                    account_id: meta.account_id.clone(),
                    attempts: meta.attempts,
                    retry_account_ids: meta.retry_account_ids.clone(),
                    route_via: ctx.resolved.via.as_str().to_string(),
                    resolved_model: ctx.resolved.model.clone(),
                    downgrade_observed: ctx.resolved.downgrade_observed,
                };
                let body_stream = audited.chain(futures_util::stream::once(async move {
                    // Hold the finalizer until the stream completes; dropping
                    // it here writes the audit entry exactly once.
                    drop(finalize);
                    Err::<Bytes, std::io::Error>(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "__end__",
                    ))
                }).filter_map(|_| async { None::<Result<Bytes, std::io::Error>> }));
                // B3: 配额响应头——配额为 0 时返回 unlimited。
                let mut builder = Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/event-stream")
                    .header("Cache-Control", "no-cache")
                    .header("X-Gateway-Tokens-Used", ctx.quota_used.to_string())
                    .header(
                        "X-Gateway-Token-Quota",
                        if ctx.token_quota == 0 {
                            "unlimited".to_string()
                        } else {
                            ctx.token_quota.to_string()
                        },
                    );
                if ctx.quota_warning {
                    builder = builder.header("X-Gateway-Quota-Warning", "90");
                }
                builder
                    .body(Body::from_stream(body_stream))
                    .unwrap_or_else(|_| {
                        GatewayError::internal("failed to build stream response").into_response()
                    })
            }
            Err(e) => {
                let status = e.kind.status().as_u16();
                state.logs.record(log_entry(status, started, 0, None, None, None));
                e.into_response()
            }
        }
    } else {
        match state.gateway.chat_completion(&upstream_req).await {
            Ok((mut resp, meta)) => {
                if let Some(u) = &resp.usage {
                    state.gateway.record_usage(&ctx.key_hash, u.total_tokens);
                }
                // B2: 响应中的 model 字段始终为客户端传入名（静默降级）。
                resp.model = client_model.clone();
                let credit = resp.usage.as_ref().and_then(|u| u.credit);
                let cached = resp.usage.as_ref().and_then(|u| u.cached_tokens());
                // B4: 非流式首字延迟——完整响应体到达时刻即首字节时刻。
                let ttft = started.elapsed().as_millis() as u64;
                let mut entry = log_entry(200, started, ttft, resp.usage.as_ref(), credit, cached);
                entry.account_id = meta.account_id.clone();
                entry.attempts = meta.attempts;
                entry.retry_account_ids = meta.retry_account_ids.clone();
                state.logs.record(entry);
                // B3: 配额响应头——配额为 0 时返回 unlimited。
                let mut builder = Response::builder().status(StatusCode::OK)
                    .header("X-Gateway-Tokens-Used", ctx.quota_used.to_string())
                    .header(
                        "X-Gateway-Token-Quota",
                        if ctx.token_quota == 0 {
                            "unlimited".to_string()
                        } else {
                            ctx.token_quota.to_string()
                        },
                    );
                if ctx.quota_warning {
                    builder = builder.header("X-Gateway-Quota-Warning", "90");
                }
                builder.body(Json(resp)).unwrap_or_else(|_| {
                    GatewayError::internal("failed to build response").into_response()
                })
            }
            Err(e) => {
                let status = e.kind.status().as_u16();
                state
                    .logs
                    .record(log_entry(status, started, 0, None, None, None));
                e.into_response()
            }
        }
    }
}

async fn handle_models(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let started = Instant::now();
    let api_key = extract_api_key(&headers);
    let src_ip = source_ip(&headers, &addr, state.trust_proxy_headers);
    // A2: 鉴权成功后若发生惰性升级，立即落盘 keys.json。
    let probe_key_name = state.gateway.models_probe(&api_key);
    if probe_key_name.is_some() {
        if !state.keys.take_dirty().is_empty() {
            if let Err(e) = state.key_store.save_keys(&state.keys.list()) {
                tracing::error!("failed to persist upgraded key hashes: {}", e);
            }
        }
    }
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
                credit: None,
                cached_tokens: None,
                source_ip: src_ip.clone(),
                key_name: probe_key_name.clone().unwrap_or_default(),
                ..Default::default()
            });
            Json(list).into_response()
        }
        Err(e) => e.into_response(),
    }
}

async fn handle_embeddings(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let started = Instant::now();
    let api_key = extract_api_key(&headers);
    let src_ip = source_ip(&headers, &addr, state.trust_proxy_headers);
    // A2: 鉴权成功后若发生惰性升级，立即落盘 keys.json。
    let probe_key_name = state.gateway.models_probe(&api_key);
    if probe_key_name.is_some() {
        if !state.keys.take_dirty().is_empty() {
            if let Err(e) = state.key_store.save_keys(&state.keys.list()) {
                tracing::error!("failed to persist upgraded key hashes: {}", e);
            }
        }
    }
    let req: EmbeddingRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return GatewayError::bad_request(format!("invalid json body: {}", e)).into_response()
        }
    };
    match state.gateway.embeddings(&req, &api_key).await {
        Ok(resp) => {
            // B4: embeddings 审计条目补齐 key_name 与 source_ip。
            state.logs.record(RequestLogEntry {
                ts: Utc::now(),
                method: "POST".to_string(),
                path: "/v1/embeddings".to_string(),
                model: String::new(),
                status: 200,
                latency_ms: started.elapsed().as_millis() as u64,
                source_ip: src_ip.clone(),
                key_name: probe_key_name.clone().unwrap_or_default(),
                ..Default::default()
            });
            Json(resp).into_response()
        }
        Err(e) => {
            let status = e.kind.status().as_u16();
            state.logs.record(RequestLogEntry {
                ts: Utc::now(),
                method: "POST".to_string(),
                path: "/v1/embeddings".to_string(),
                model: String::new(),
                status,
                latency_ms: started.elapsed().as_millis() as u64,
                source_ip: src_ip.clone(),
                key_name: probe_key_name.clone().unwrap_or_default(),
                ..Default::default()
            });
            e.into_response()
        }
    }
}
