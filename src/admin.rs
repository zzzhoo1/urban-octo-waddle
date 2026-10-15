use std::sync::Arc;

use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::account::{Account, AccountData, AccountStatus};
use crate::error::GatewayError;
use crate::AppState;

/// Admin API router (all routes behind key auth).
pub fn admin_router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/admin/api/accounts", get(list_accounts).post(create_account))
        .route("/admin/api/accounts/{id}", patch(patch_account).delete(delete_account))
        .route("/admin/api/status", get(status))
        .route("/admin/api/logs", get(list_logs))
        .route("/admin/api/info", get(info))
        .route("/admin/api/oauth/start", post(oauth_start))
        .route("/admin/api/oauth/callback", get(oauth_callback))
        .route_layer(middleware::from_fn_with_state(state.clone(), admin_auth));
    Router::new().merge(api).with_state(state)
}

/// Key auth for the admin plane: Authorization Bearer or gw_key cookie.
async fn admin_auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let mut token = String::new();
    if let Some(auth) = req.headers().get(axum::http::header::AUTHORIZATION) {
        if let Ok(value) = auth.to_str() {
            if let Some(stripped) = value.strip_prefix("Bearer ") {
                token = stripped.to_string();
            }
        }
    }
    if token.is_empty() {
        if let Some(cookie) = req.headers().get(axum::http::header::COOKIE) {
            if let Ok(value) = cookie.to_str() {
                for pair in value.split(';') {
                    let pair = pair.trim();
                    if let Some(v) = pair.strip_prefix("gw_key=") {
                        token = v.to_string();
                        break;
                    }
                }
            }
        }
    }
    if !state.keys.authenticate(&token) {
        return (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", "Bearer")],
            "unauthorized",
        )
            .into_response();
    }
    next.run(req).await
}

/// Account DTO returned to the panel (credential never leaves the server).
#[derive(Debug, Serialize)]
struct AccountDto {
    id: String,
    name: String,
    status: String,
    failure_count: u32,
    last_failure_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn to_dto(data: &AccountData) -> AccountDto {
    AccountDto {
        id: data.id.clone(),
        name: data.name.clone(),
        status: data.status.as_str().to_string(),
        failure_count: data.failure_count,
        last_failure_at: data.last_failure_at,
        created_at: data.created_at,
    }
}

async fn list_accounts(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let dtos: Vec<AccountDto> = state.pool.list_data().iter().map(to_dto).collect();
    Json(dtos)
}

#[derive(Debug, Deserialize)]
struct CreateAccountReq {
    name: String,
    credential: String,
}

async fn create_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateAccountReq>,
) -> Response {
    if req.name.is_empty() || req.credential.is_empty() {
        return GatewayError::bad_request("name and credential are required").into_response();
    }
    let enc = match state.encryptor.encrypt(&req.credential) {
        Ok(e) => e,
        Err(e) => return GatewayError::internal(format!("encrypt failed: {}", e)).into_response(),
    };
    let id = format!("acc-{}", chrono::Utc::now().format("%Y%m%d%H%M%S%.6f"));
    let account = Account::new(id, req.name, enc);
    let dto = to_dto(&account.data());
    state.pool.add(account);
    persist(&state);
    (StatusCode::CREATED, Json(dto)).into_response()
}

#[derive(Debug, Deserialize)]
struct PatchAccountReq {
    status: String,
}

async fn patch_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PatchAccountReq>,
) -> Response {
    let Some(account) = state.pool.get(&id) else {
        return GatewayError::bad_request("account not found").into_response();
    };
    match req.status.as_str() {
        "active" => account.enable(),
        "disabled" => account.disable(),
        _ => return GatewayError::bad_request("invalid status").into_response(),
    }
    persist(&state);
    Json(to_dto(&account.data())).into_response()
}

async fn delete_account(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    if !state.pool.remove(&id) {
        return GatewayError::bad_request("account not found").into_response();
    }
    persist(&state);
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Debug, Serialize)]
struct StatusDto {
    id: String,
    name: String,
    status: String,
    healthy: bool,
    failure_count: u32,
}

async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let out: Vec<StatusDto> = state
        .pool
        .list()
        .iter()
        .map(|a| StatusDto {
            id: a.id(),
            name: a.name(),
            status: a.status().to_string(),
            healthy: a.is_healthy(),
            failure_count: a.failure_count(),
        })
        .collect();
    Json(out)
}

async fn list_logs(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.logs.list())
}

#[derive(Debug, Serialize)]
struct InfoDto {
    version: String,
    listen: String,
    circuit_threshold: u32,
    circuit_cooldown_secs: u64,
    session_ttl_secs: u64,
    session_max_size: usize,
    failure_threshold: u32,
    retry_max: u32,
    retry_backoff_ms: u64,
    account_recover_after_secs: u64,
    default_model: String,
    sticky_sessions: usize,
    api_keys: usize,
    upstream: String,
}

async fn info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(InfoDto {
        version: env!("CARGO_PKG_VERSION").to_string(),
        listen: state.listen.clone(),
        circuit_threshold: state.cfg.circuit_threshold,
        circuit_cooldown_secs: state.cfg.circuit_cooldown.as_secs(),
        session_ttl_secs: state.cfg.session_ttl.as_secs(),
        session_max_size: state.cfg.session_max_size,
        failure_threshold: state.cfg.failure_threshold,
        retry_max: state.cfg.retry_max,
        retry_backoff_ms: state.cfg.retry_backoff.as_millis() as u64,
        account_recover_after_secs: state.cfg.account_recover_after.as_secs(),
        default_model: state.cfg.default_model.clone(),
        sticky_sessions: state.gateway.session_size(),
        api_keys: state.keys.count(),
        upstream: crate::upstream::CODEARTS_GATEWAY_PROD.to_string(),
    })
}

async fn oauth_start() -> Response {
    // OAuth is optional in the legacy gateway; it is not configured here.
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"error": "oauth not configured"})),
    )
        .into_response()
}

async fn oauth_callback() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"error": "oauth not configured"})),
    )
        .into_response()
}

/// Persist the pool; failures are logged but do not fail the request
/// (same behaviour as the legacy gateway's best-effort save).
fn persist(state: &Arc<AppState>) {
    if let Err(e) = state.store.save(&state.pool.list_data()) {
        tracing::error!("failed to persist accounts: {}", e);
    }
}

/// Keep AccountStatus referenced for serde round-trips in tests.
#[allow(dead_code)]
fn _status_roundtrip(s: AccountStatus) -> String {
    s.to_string()
}
