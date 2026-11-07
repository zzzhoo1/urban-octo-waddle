use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use crate::account::Pool;
use crate::auth::{AuthFailure, KeyAuth};
use crate::circuit::CircuitRegistry;
use crate::config::GatewayConfig;
use crate::error::{retryable, GatewayError};
use crate::openai::{
    ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest, EmbeddingResponse, ModelList,
};
use crate::router::{ModelRouter, Resolved};
use crate::session::SessionManager;
use crate::upstream::{ByteStream, CodeArtsClient};

/// B4: upstream call outcome metadata for audit attribution.
#[derive(Debug, Clone, Default)]
pub struct CallMeta {
    /// Account that served the request (final attempt).
    pub account_id: String,
    /// Total attempts including the first (>= 1).
    pub attempts: u32,
    /// Account ids tried in order.
    pub retry_account_ids: Vec<String>,
}

/// Successful auth + routing decision for one request.
pub struct RequestContext {
    pub key_name: String,
    pub key_hash: String,
    pub resolved: Resolved,
    /// B3: quota info for X-Gateway-* response headers.
    pub quota_used: u64,
    pub token_quota: u64,
    pub quota_warning: bool,
}

/// Core dispatch gateway: auth -> validate -> route -> pool select ->
/// circuit check -> upstream call with retries/backoff -> bookkeeping.
pub struct Gateway {
    pool: Arc<Pool>,
    upstream: Arc<CodeArtsClient>,
    keys: Arc<KeyAuth>,
    circuit: CircuitRegistry,
    sessions: SessionManager,
    cfg: GatewayConfig,
    router: Arc<ModelRouter>,
}

impl Gateway {
    pub fn new(
        pool: Arc<Pool>,
        upstream: Arc<CodeArtsClient>,
        keys: Arc<KeyAuth>,
        cfg: GatewayConfig,
        router: Arc<ModelRouter>,
    ) -> Self {
        Self {
            pool,
            upstream,
            keys,
            circuit: CircuitRegistry::new(),
            sessions: SessionManager::new(cfg.session_ttl, cfg.session_max_size),
            cfg,
            router,
        }
    }

    /// Authenticate a data-plane request and resolve its model routing.
    /// B1: allowlist matches the client-supplied name OR the resolved
    /// upstream model name (either hit passes); the check uses the
    /// pre-downgrade resolution so a downgrade never fails auth.
    pub fn authorize(&self, api_key: &str, model: &str, is_background: bool) -> Result<RequestContext, GatewayError> {
        // B1: 先做基础解析（用户规则 -> 内置档位 -> 透传），白名单比对
        // 客户端传入名与解析后模型名任一命中。
        let base = self.router.resolve_base(model, "");
        let identity = self
            .keys
            .authenticate_for_model(api_key, model, &base.model)
            .map_err(|f| match f {
                AuthFailure::UnknownKey => GatewayError::unauthorized("invalid api key"),
                AuthFailure::Expired => GatewayError::unauthorized("api key expired"),
                AuthFailure::Disabled => GatewayError::unauthorized("api key disabled"),
                AuthFailure::QuotaExceeded => {
                    GatewayError::rate_limited("token quota exceeded for this key")
                }
                AuthFailure::ModelNotAllowed => {
                    GatewayError::unauthorized("model not allowed for this key")
                }
                AuthFailure::Forbidden => {
                    GatewayError::unauthorized("admin privilege required")
                        .with_status(axum::http::StatusCode::FORBIDDEN)
                }
            })?;
        // A2: 旧 SHA-256 条目在成功鉴权后惰性升级为 argon2id，失败不影响请求。
        self.keys.lazy_upgrade(api_key);
        // B3: 鉴权时惰性翻转配额周期边界。
        self.keys.roll_quota_periods();
        // B2: 降级叠加在基础解析之上（log_only 只观察不改写）。
        let resolved = self.router.apply_downgrade(base, is_background);
        // B3: 配额信息用于 X-Gateway-* 响应头与软阈值预警。
        let (quota_used, token_quota, quota_warning) = self.keys.quota_snapshot(&self.keys.fingerprint_of(api_key));
        if quota_warning {
            tracing::warn!(key = %identity.key_name, "quota soft threshold (90%) reached");
        }
        Ok(RequestContext {
            key_name: identity.key_name,
            // 记账索引与密钥表一致，使用确定性指纹而非明文摘要。
            key_hash: self.keys.fingerprint_of(api_key),
            resolved,
            quota_used,
            token_quota,
            quota_warning,
        })
    }

    /// Record token usage against a key after a completed request.
    pub fn record_usage(&self, fingerprint: &str, tokens: u64) {
        self.keys.record_usage(fingerprint, tokens);
    }

    /// A2/B4: 数据面鉴权探测——校验密钥（user/admin 均可）、触发旧摘要
    /// 惰性升级与周期翻转，返回 Some(key_name)（鉴权成功）或 None（失败）；
    /// 若有待落盘变更，由调用方落盘 keys.json。
    pub fn models_probe(&self, api_key: &str) -> Option<String> {
        let identity = self
            .keys
            .authenticate_for_model(api_key, "", "")
            .ok()?;
        self.keys.lazy_upgrade(api_key);
        self.keys.roll_quota_periods();
        Some(identity.key_name)
    }

    fn retry_delay(&self, attempt: u32) -> Duration {
        if self.cfg.retry_backoff.is_zero() {
            Duration::ZERO
        } else {
            self.cfg.retry_backoff * (attempt + 1)
        }
    }

    pub async fn chat_completion(
        &self,
        req: &ChatCompletionRequest,
    ) -> Result<(ChatCompletionResponse, CallMeta), GatewayError> {
        req.validate()?;
        let mut last_err = GatewayError::unavailable("no account available");
        let mut tried: Vec<String> = Vec::new();
        for attempt in 0..=self.cfg.retry_max {
            let account = match self.pool.select(Utc::now(), self.cfg.account_recover_after) {
                Some(acc) => acc,
                None => return Err(GatewayError::unavailable("no healthy account available")),
            };
            tried.push(account.id());
            let breaker = self.circuit.get_or_create(
                &account.id(),
                self.cfg.circuit_threshold,
                self.cfg.circuit_cooldown,
            );
            if !breaker.allow() {
                continue;
            }
            match self.upstream.chat_completion(req).await {
                Ok(resp) => {
                    breaker.record_success();
                    account.reset_failures();
                    let meta = CallMeta {
                        account_id: account.id(),
                        attempts: tried.len() as u32,
                        retry_account_ids: tried.clone(),
                    };
                    return Ok((resp, meta));
                }
                Err(e) => {
                    tracing::debug!("[upstream-debug] chat_completion error: {} (attempt {})", e, attempt);
                    breaker.record_failure();
                    account.mark_failure(Utc::now(), self.cfg.failure_threshold);
                    last_err = e;
                    if !retryable(&last_err) {
                        return Err(last_err);
                    }
                }
            }
            tokio::time::sleep(self.retry_delay(attempt)).await;
        }
        Err(last_err)
    }

    pub async fn chat_completion_stream(
        &self,
        req: &ChatCompletionRequest,
        session_id: &str,
    ) -> Result<(ByteStream, CallMeta), GatewayError> {
        req.validate()?;
        let mut last_err = GatewayError::unavailable("no account available");
        let mut tried: Vec<String> = Vec::new();
        for attempt in 0..=self.cfg.retry_max {
            let account = if !session_id.is_empty() {
                match self.sessions.get(session_id) {
                    Some(bound_id) => self
                        .pool
                        .get(&bound_id)
                        .filter(|a| a.is_healthy())
                        .or_else(|| self.pool.select(Utc::now(), self.cfg.account_recover_after)),
                    None => self.pool.select(Utc::now(), self.cfg.account_recover_after),
                }
            } else {
                self.pool.select(Utc::now(), self.cfg.account_recover_after)
            };
            let Some(account) = account else {
                return Err(GatewayError::unavailable("no healthy account available"));
            };
            tried.push(account.id());
            let breaker = self.circuit.get_or_create(
                &account.id(),
                self.cfg.circuit_threshold,
                self.cfg.circuit_cooldown,
            );
            if !breaker.allow() {
                continue;
            }
            match self.upstream.chat_completion_stream(req).await {
                Ok(stream) => {
                    breaker.record_success();
                    account.reset_failures();
                    if !session_id.is_empty() {
                        self.sessions.bind(session_id, &account.id());
                    }
                    let meta = CallMeta {
                        account_id: account.id(),
                        attempts: tried.len() as u32,
                        retry_account_ids: tried.clone(),
                    };
                    return Ok((stream, meta));
                }
                Err(e) => {
                    tracing::debug!("[upstream-debug] chat_completion_stream error: {} (attempt {})", e, attempt);
                    breaker.record_failure();
                    account.mark_failure(Utc::now(), self.cfg.failure_threshold);
                    last_err = e;
                    if !retryable(&last_err) {
                        return Err(last_err);
                    }
                }
            }
            tokio::time::sleep(self.retry_delay(attempt)).await;
        }
        Err(last_err)
    }

    pub async fn models(&self, api_key: &str) -> Result<ModelList, GatewayError> {
        if !self.keys.authenticate(api_key) {
            return Err(GatewayError::unauthorized("invalid api key"));
        }
        // A2: 任何数据面成功鉴权都触发旧摘要惰性升级。
        self.keys.lazy_upgrade(api_key);
        self.upstream.models().await
    }

    pub async fn embeddings(
        &self,
        _req: &EmbeddingRequest,
        api_key: &str,
    ) -> Result<EmbeddingResponse, GatewayError> {
        if !self.keys.authenticate(api_key) {
            return Err(GatewayError::unauthorized("invalid api key"));
        }
        // A2: 任何数据面成功鉴权都触发旧摘要惰性升级。
        self.keys.lazy_upgrade(api_key);
        Err(GatewayError::bad_gateway(
            "codearts upstream does not support embeddings",
        ))
    }

    pub fn session_size(&self) -> usize {
        self.sessions.size()
    }
}
