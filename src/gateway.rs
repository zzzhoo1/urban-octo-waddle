use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use crate::account::Pool;
use crate::auth::KeyAuth;
use crate::circuit::CircuitRegistry;
use crate::config::GatewayConfig;
use crate::error::{retryable, GatewayError};
use crate::openai::{
    ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest, EmbeddingResponse, ModelList,
};
use crate::session::SessionManager;
use crate::upstream::{ByteStream, CodeArtsClient};

/// Core dispatch gateway: auth -> validate -> pool select -> circuit check
/// -> upstream call with retries/backoff -> success/failure bookkeeping.
pub struct Gateway {
    pool: Arc<Pool>,
    upstream: Arc<CodeArtsClient>,
    keys: Arc<KeyAuth>,
    circuit: CircuitRegistry,
    sessions: SessionManager,
    cfg: GatewayConfig,
}

impl Gateway {
    pub fn new(
        pool: Arc<Pool>,
        upstream: Arc<CodeArtsClient>,
        keys: Arc<KeyAuth>,
        cfg: GatewayConfig,
    ) -> Self {
        Self {
            pool,
            upstream,
            keys,
            circuit: CircuitRegistry::new(),
            sessions: SessionManager::new(cfg.session_ttl, cfg.session_max_size),
            cfg,
        }
    }

    pub fn authenticate(&self, api_key: &str) -> Result<(), GatewayError> {
        if self.keys.authenticate(api_key) {
            Ok(())
        } else {
            Err(GatewayError::unauthorized("invalid api key"))
        }
    }

    /// Retry delay before the attempt-th retry (0-based): (attempt+1) * backoff.
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
        api_key: &str,
    ) -> Result<ChatCompletionResponse, GatewayError> {
        self.authenticate(api_key)?;
        req.validate()?;

        let mut last_err = GatewayError::unavailable("no account available");
        for attempt in 0..=self.cfg.retry_max {
            let account = match self.pool.select(Utc::now(), self.cfg.account_recover_after) {
                Some(acc) => acc,
                None => return Err(GatewayError::unavailable("no healthy account available")),
            };
            let breaker = self.circuit.get_or_create(
                &account.id(),
                self.cfg.circuit_threshold,
                self.cfg.circuit_cooldown,
            );
            if !breaker.allow() {
                continue; // circuit open: try the next account
            }
            match self.upstream.chat_completion(req).await {
                Ok(resp) => {
                    breaker.record_success();
                    account.reset_failures();
                    return Ok(resp);
                }
                Err(e) => {
                    tracing::debug!(
                        "[upstream-debug] chat_completion error: {} (attempt {})",
                        e,
                        attempt
                    );
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
        api_key: &str,
    ) -> Result<ByteStream, GatewayError> {
        self.authenticate(api_key)?;
        req.validate()?;

        let mut last_err = GatewayError::unavailable("no account available");
        for attempt in 0..=self.cfg.retry_max {
            // Sticky session: prefer the account bound to this session.
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
                    return Ok(stream);
                }
                Err(e) => {
                    tracing::debug!(
                        "[upstream-debug] chat_completion_stream error: {} (attempt {})",
                        e,
                        attempt
                    );
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
        self.authenticate(api_key)?;
        self.upstream.models().await
    }

    pub async fn embeddings(
        &self,
        _req: &EmbeddingRequest,
        api_key: &str,
    ) -> Result<EmbeddingResponse, GatewayError> {
        self.authenticate(api_key)?;
        Err(GatewayError::bad_gateway(
            "codearts upstream does not support embeddings",
        ))
    }

    pub fn session_size(&self) -> usize {
        self.sessions.size()
    }
}
