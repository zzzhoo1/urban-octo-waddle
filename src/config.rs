use std::time::Duration;

/// Gateway runtime configuration.
/// Defaults match the legacy Go gateway (cmd/gateway/main.go).
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Consecutive upstream failures before a circuit opens.
    pub circuit_threshold: u32,
    /// How long an open circuit waits before allowing a probe.
    pub circuit_cooldown: Duration,
    /// Sticky session TTL.
    pub session_ttl: Duration,
    /// Max sticky sessions kept in memory.
    pub session_max_size: usize,
    /// Consecutive failures before an account is put into cooling.
    pub failure_threshold: u32,
    /// Max retry attempts per request (in addition to the first try).
    pub retry_max: u32,
    /// Default model name used when upstream omits it.
    pub default_model: String,
    /// Cooling accounts recover automatically after this duration.
    pub account_recover_after: Duration,
    /// Linear backoff base before the n-th retry (delay = (n+1) * base).
    pub retry_backoff: Duration,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            circuit_threshold: 20,
            circuit_cooldown: Duration::from_secs(30),
            session_ttl: Duration::from_secs(10 * 60),
            session_max_size: 1000,
            failure_threshold: 20,
            retry_max: 3,
            default_model: "codearts-pro".to_string(),
            account_recover_after: Duration::from_secs(60),
            retry_backoff: Duration::from_millis(500),
        }
    }
}
