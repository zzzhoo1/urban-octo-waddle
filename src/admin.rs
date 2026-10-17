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
use crate::auth::{fingerprint_key, hash_key_slow, mask_key, ApiKey, AuthFailure, KeyRole};
use crate::error::GatewayError;
use crate::router::{DowngradeRule, RouteRule};
use crate::AppState;

/// Admin API router (all routes behind key auth).
pub fn admin_router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/admin/api/accounts", get(list_accounts).post(create_account))
        .route("/admin/api/accounts/{id}", patch(patch_account).delete(delete_account))
        .route("/admin/api/status", get(status))
        .route("/admin/api/logs", get(list_logs))
        .route("/admin/api/stats", get(stats))
        .route("/admin/api/info", get(info))
        .route("/admin/api/keys", get(list_keys).post(create_key))
        .route("/admin/api/keys/{name}", patch(patch_key).delete(delete_key))
        .route("/admin/api/keys/{name}/reset-usage", post(reset_key_usage))
        .route("/admin/api/router", get(get_router).put(put_router))
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
    // A2 鉴权分级：无效/过期/禁用密钥返回 401；有效但非 admin 角色的
    // 密钥返回 403 + unauthorized 信封，面板据此区分『未登录』与『权限不足』。
    match state.keys.authenticate_admin(&token) {
        Ok(identity) => {
            let mut req = req;
            req.extensions_mut().insert(identity);
            next.run(req).await
        }
        Err(AuthFailure::Forbidden) => GatewayError::unauthorized("admin privilege required")
            .with_status(StatusCode::FORBIDDEN)
            .into_response(),
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", "Bearer")],
            "unauthorized",
        )
            .into_response(),
    }
}

/// Admin-plane actor extracted from request extensions (inserted by the
/// auth middleware). Used for A2 write-operation audit logging; never
/// carries key plaintext or credentials.
#[derive(Debug, Clone, Default)]
struct AdminActor {
    key_name: String,
    source_ip: String,
}

impl AdminActor {
    fn audit(&self, action: &str, target: &str) {
        tracing::info!(
            actor_key_name = %self.key_name,
            action = %action,
            target = %target,
            source_ip = %self.source_ip,
            "admin write"
        );
    }
}

impl axum::extract::FromRequestParts<Arc<AppState>> for AdminActor {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let key_name = parts
            .extensions
            .get::<crate::auth::AuthIdentity>()
            .map(|i| i.key_name.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let source_ip = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        Ok(AdminActor { key_name, source_ip })
    }
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
    actor: AdminActor,
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
    let account = Account::new(id, req.name.clone(), enc);
    let dto = to_dto(&account.data());
    state.pool.add(account);
    persist(&state);
    actor.audit("create_account", &req.name);
    (StatusCode::CREATED, Json(dto)).into_response()
}

#[derive(Debug, Deserialize)]
struct PatchAccountReq {
    status: String,
}

async fn patch_account(
    State(state): State<Arc<AppState>>,
    actor: AdminActor,
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
    actor.audit("patch_account", &id);
    Json(to_dto(&account.data())).into_response()
}

async fn delete_account(
    State(state): State<Arc<AppState>>,
    actor: AdminActor,
    Path(id): Path<String>,
) -> Response {
    if !state.pool.remove(&id) {
        return GatewayError::bad_request("account not found").into_response();
    }
    persist(&state);
    actor.audit("delete_account", &id);
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

/// B4: query params shared by logs and stats endpoints.
#[derive(Debug, Deserialize)]
struct LogQueryParams {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    key_name: Option<String>,
    #[serde(default)]
    status_class: Option<String>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    before: Option<String>,
}

fn parse_ts(v: &Option<String>) -> Result<Option<chrono::DateTime<chrono::Utc>>, Response> {
    match v {
        None => Ok(None),
        Some(s) => match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(t) => Ok(Some(t.with_timezone(&chrono::Utc))),
            Err(_) => Err(GatewayError::bad_request(format!("invalid timestamp: {}", s)).into_response()),
        },
    }
}

fn build_log_query(p: &LogQueryParams) -> Result<crate::logs::LogQuery, Response> {
    let from = parse_ts(&p.from)?;
    let to = parse_ts(&p.to)?;
    let before = parse_ts(&p.before)?;
    if let Some(class) = &p.status_class {
        if !matches!(class.as_str(), "2xx" | "4xx" | "5xx") {
            return Err(GatewayError::bad_request(format!("invalid status_class: {}", class)).into_response());
        }
    }
    Ok(crate::logs::LogQuery {
        from,
        to,
        model: p.model.clone().filter(|s| !s.is_empty()),
        key_name: p.key_name.clone().filter(|s| !s.is_empty()),
        status_class: p.status_class.clone().filter(|s| !s.is_empty()),
        limit: p.limit,
        before,
    })
}

async fn list_logs(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(p): axum::extract::Query<LogQueryParams>,
) -> Response {
    match build_log_query(&p) {
        Ok(q) => Json(state.logs.list(&q)).into_response(),
        Err(resp) => resp,
    }
}

/// Usage statistics for the dashboard (totals, by-day, by-model, by-key).
async fn stats(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(p): axum::extract::Query<LogQueryParams>,
) -> Response {
    match build_log_query(&p) {
        Ok(q) => Json(state.logs.stats(&q)).into_response(),
        Err(resp) => resp,
    }
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
    admin_keys: usize,
    user_keys: usize,
    /// A3: 上游连接建立超时（秒）。
    upstream_connect_timeout_secs: u64,
    /// A3: 流式首字节超时（秒），首帧后不受限。
    stream_first_byte_timeout_secs: u64,
    /// A3: 非流式整体超时（秒）。
    non_stream_timeout_secs: u64,
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
        admin_keys: state.keys.count_by_role(KeyRole::Admin),
        user_keys: state.keys.count_by_role(KeyRole::User),
        upstream_connect_timeout_secs: 10,
        stream_first_byte_timeout_secs: 30,
        non_stream_timeout_secs: 120,
        upstream: crate::upstream::CODEARTS_GATEWAY_PROD.to_string(),
    })
}

// ---- Multi-key management ----

#[derive(Debug, Serialize)]
struct KeyDto {
    name: String,
    hint: String,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    token_quota: u64,
    tokens_used: u64,
    model_allowlist: Vec<String>,
    enabled: bool,
    expired: bool,
    /// A2: admin 或 user；旧数据缺省读作 admin。
    role: String,
    /// A2: 命令行注入的主密钥，为真时不可删改。
    is_master: bool,
    /// A2: 仍为无盐 SHA-256 摘要时为真，面板提示轮换。
    legacy_hash: bool,
    /// B3: 配额周期（none/daily/monthly）。
    quota_period: String,
    /// B3: 当前周期起始时刻（UTC）。
    quota_period_start: Option<chrono::DateTime<chrono::Utc>>,
    /// B3: 周期内已用 token。
    tokens_used_in_period: u64,
}

fn key_dto(k: &ApiKey) -> KeyDto {
    KeyDto {
        name: k.name.clone(),
        hint: k.hint.clone(),
        created_at: k.created_at,
        expires_at: k.expires_at,
        token_quota: k.token_quota,
        tokens_used: k.tokens_used,
        model_allowlist: k.model_allowlist.clone(),
        enabled: k.enabled,
        expired: k.is_expired(chrono::Utc::now()),
        role: k.role.as_str().to_string(),
        is_master: k.is_master,
        legacy_hash: k.legacy_hash,
        quota_period: k.quota_period.as_str().to_string(),
        quota_period_start: k.quota_period_start,
        tokens_used_in_period: k.tokens_used_in_period,
    }
}

async fn list_keys(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut dtos: Vec<KeyDto> = state.keys.list().iter().map(key_dto).collect();
    dtos.sort_by(|a, b| a.name.cmp(&b.name));
    Json(dtos)
}

#[derive(Debug, Deserialize)]
struct CreateKeyReq {
    name: String,
    key: String,
    #[serde(default)]
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    token_quota: u64,
    #[serde(default)]
    model_allowlist: Vec<String>,
    /// A2: 新建密钥默认 user 角色，需显式传 admin 才创建管理密钥。
    #[serde(default)]
    role: Option<String>,
    /// B3: 配额周期，缺省 none（累计语义）。
    #[serde(default)]
    quota_period: Option<String>,
}

async fn create_key(
    State(state): State<Arc<AppState>>,
    actor: AdminActor,
    Json(req): Json<CreateKeyReq>,
) -> Response {
    if req.name.is_empty() || req.key.is_empty() {
        return GatewayError::bad_request("name and key are required").into_response();
    }
    if state.keys.get_by_name(&req.name).is_some() {
        return GatewayError::bad_request("key name already exists").into_response();
    }
    // A2: 未知角色取值按 user 处理并记录 WARN。
    let role = match req.role.as_deref() {
        None | Some("") | Some("user") => KeyRole::User,
        Some("admin") => KeyRole::Admin,
        Some(other) => {
            tracing::warn!(role = %other, "unknown key role, treated as user");
            KeyRole::User
        }
    };
    // B3: 未知配额周期取值返回 400。
    let quota_period = match req.quota_period.as_deref() {
        None | Some("") | Some("none") => crate::auth::QuotaPeriod::None,
        Some("daily") => crate::auth::QuotaPeriod::Daily,
        Some("monthly") => crate::auth::QuotaPeriod::Monthly,
        Some(other) => {
            return GatewayError::bad_request(format!("unknown quota_period: {}", other))
                .into_response()
        }
    };
    let entry = ApiKey {
        name: req.name.clone(),
        key_hash: hash_key_slow(&req.key),
        fingerprint: fingerprint_key(&state.secret, &req.key),
        hint: mask_key(&req.key),
        created_at: Some(chrono::Utc::now()),
        expires_at: req.expires_at,
        token_quota: req.token_quota,
        tokens_used: 0,
        model_allowlist: req.model_allowlist,
        enabled: true,
        role,
        is_master: false,
        legacy_hash: false,
        quota_period,
        quota_period_start: if quota_period == crate::auth::QuotaPeriod::None {
            None
        } else {
            Some(quota_period.period_start(chrono::Utc::now()))
        },
        tokens_used_in_period: 0,
    };
    let dto = key_dto(&entry);
    state.keys.upsert(entry);
    persist_keys(&state);
    actor.audit("create_key", &dto.name);
    (StatusCode::CREATED, Json(dto)).into_response()
}

#[derive(Debug, Deserialize)]
struct PatchKeyReq {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    token_quota: Option<u64>,
    /// B3: 显式置空语义——字段出现且为 null 表示清除过期时间（永不过期），
    /// 字段缺省表示不修改。用自定义枚举区分三态。
    #[serde(default, deserialize_with = "deserialize_optional_datetime")]
    expires_at: PatchField<chrono::DateTime<chrono::Utc>>,
    /// A2: 可选角色变更；主密钥任何修改均拒绝。
    #[serde(default)]
    role: Option<String>,
    /// B3: 模型白名单可修改；空数组 = 放开为全部模型；缺省不修改。
    #[serde(default)]
    model_allowlist: Option<Vec<String>>,
    /// B3: 配额周期可修改；缺省不修改。
    #[serde(default)]
    quota_period: Option<String>,
}

/// B3: tri-state patch field — Absent (unchanged), Null (clear), Set(value).
#[derive(Debug, Clone, Default)]
enum PatchField<T> {
    #[default]
    Absent,
    Null,
    Set(T),
}

fn deserialize_optional_datetime<'de, D>(d: D) -> Result<PatchField<chrono::DateTime<chrono::Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<chrono::DateTime<chrono::Utc>>::deserialize(d)?;
    Ok(match v {
        None => PatchField::Null,
        Some(t) => PatchField::Set(t),
    })
}

async fn patch_key(
    State(state): State<Arc<AppState>>,
    actor: AdminActor,
    Path(name): Path<String>,
    Json(req): Json<PatchKeyReq>,
) -> Response {
    let Some(mut key) = state.keys.get_by_name(&name) else {
        return GatewayError::bad_request("key not found").into_response();
    };
    // A2: 主密钥不可通过面板降级或修改。
    if key.is_master {
        return GatewayError::bad_request("master key cannot be modified").into_response();
    }
    if let Some(e) = req.enabled {
        key.enabled = e;
    }
    if let Some(q) = req.token_quota {
        key.token_quota = q;
    }
    // B3: 显式置空语义——null 清除过期时间，Set 更新，Absent 不修改。
    match req.expires_at {
        PatchField::Null => key.expires_at = None,
        PatchField::Set(t) => key.expires_at = Some(t),
        PatchField::Absent => {}
    }
    // B3: 白名单可修改，空数组 = 全部模型。
    if let Some(list) = req.model_allowlist {
        key.model_allowlist = list;
    }
    // B3: 配额周期可修改；切换周期时重置周期起点与周期内用量。
    if let Some(p) = req.quota_period.as_deref() {
        let period = match crate::auth::QuotaPeriod::parse(p) {
            Some(v) => v,
            None => {
                return GatewayError::bad_request(format!("unknown quota_period: {}", p))
                    .into_response()
            }
        };
        if period != key.quota_period {
            key.quota_period = period;
            key.quota_period_start = if period == crate::auth::QuotaPeriod::None {
                None
            } else {
                Some(period.period_start(chrono::Utc::now()))
            };
            key.tokens_used_in_period = 0;
        }
    }
    if let Some(r) = req.role.as_deref() {
        match r {
            "admin" => key.role = KeyRole::Admin,
            "user" => key.role = KeyRole::User,
            other => {
                tracing::warn!(role = %other, "unknown key role, treated as user");
                key.role = KeyRole::User;
            }
        }
    }
    let dto = key_dto(&key);
    state.keys.upsert(key);
    persist_keys(&state);
    actor.audit("patch_key", &name);
    Json(dto).into_response()
}

/// B3: 重置当前周期用量（累计值不变）。
async fn reset_key_usage(
    State(state): State<Arc<AppState>>,
    actor: AdminActor,
    Path(name): Path<String>,
) -> Response {
    let Some(mut key) = state.keys.get_by_name(&name) else {
        return GatewayError::bad_request("key not found").into_response();
    };
    key.tokens_used_in_period = 0;
    if key.quota_period != crate::auth::QuotaPeriod::None {
        key.quota_period_start = Some(key.quota_period.period_start(chrono::Utc::now()));
    }
    let dto = key_dto(&key);
    state.keys.upsert(key);
    persist_keys(&state);
    actor.audit("reset_key_usage", &name);
    Json(dto).into_response()
}

async fn delete_key(
    State(state): State<Arc<AppState>>,
    actor: AdminActor,
    Path(name): Path<String>,
) -> Response {
    // A2: 主密钥不可删除。
    if let Some(key) = state.keys.get_by_name(&name) {
        if key.is_master {
            return GatewayError::bad_request("master key cannot be modified").into_response();
        }
    }
    if !state.keys.remove_by_name(&name) {
        return GatewayError::bad_request("key not found").into_response();
    }
    persist_keys(&state);
    actor.audit("delete_key", &name);
    StatusCode::NO_CONTENT.into_response()
}

// ---- Router rules management ----

async fn get_router(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.router.config())
}

#[derive(Debug, Deserialize)]
struct PutRouterReq {
    #[serde(default)]
    rules: Vec<RouteRule>,
    #[serde(default)]
    downgrade: Option<DowngradeRule>,
}

async fn put_router(
    State(state): State<Arc<AppState>>,
    actor: AdminActor,
    Json(req): Json<PutRouterReq>,
) -> Response {
    // B1: 校验前移——保存前校验互斥/必填/glob 语法/别名去重，
    // 非法时整体原子拒绝（不做部分保存），message 指明规则序号与原因。
    let new_cfg = crate::router::RouterConfig {
        rules: req.rules,
        downgrade: req.downgrade,
    };
    if let Err(reason) = crate::router::validate_config(&new_cfg) {
        return GatewayError::bad_request(reason).into_response();
    }
    let rule_count = new_cfg.rules.len();
    state.router.replace(new_cfg);
    persist_router(&state);
    actor.audit("put_router", &format!("{} rules", rule_count));
    Json(state.router.config()).into_response()
}

async fn oauth_start() -> Response {
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

/// Persist the account pool; failures are logged but do not fail the request.
fn persist(state: &Arc<AppState>) {
    if let Err(e) = state.store.save(&state.pool.list_data()) {
        tracing::error!("failed to persist accounts: {}", e);
    }
}

/// Persist API keys to keys.json next to the account store.
fn persist_keys(state: &Arc<AppState>) {
    if let Err(e) = state.key_store.save_keys(&state.keys.list()) {
        tracing::error!("failed to persist keys: {}", e);
    }
}

/// Persist router config to router.json next to the account store.
fn persist_router(state: &Arc<AppState>) {
    if let Err(e) = state.key_store.save_router(&state.router.config()) {
        tracing::error!("failed to persist router: {}", e);
    }
}

/// Keep AccountStatus referenced for serde round-trips in tests.
#[allow(dead_code)]
fn _status_roundtrip(s: AccountStatus) -> String {
    s.to_string()
}
