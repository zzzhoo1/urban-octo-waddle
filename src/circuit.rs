use dashmap::DashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Circuit breaker state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    Open,
    HalfOpen,
}

/// Per-account circuit breaker.
///
/// - closed: requests flow; `threshold` consecutive failures opens the breaker.
/// - open: requests rejected; after `cooldown` one probe is allowed (half-open).
/// - half_open: exactly one probe in flight; success closes, failure re-opens.
#[derive(Debug)]
pub struct Breaker {
    threshold: u32,
    cooldown: Duration,
    inner: RwLock<Inner>,
}

#[derive(Debug)]
struct Inner {
    state: State,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
    probe_pending: bool,
}

impl Breaker {
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            threshold,
            cooldown,
            inner: RwLock::new(Inner {
                state: State::Closed,
                consecutive_failures: 0,
                opened_at: None,
                probe_pending: false,
            }),
        }
    }

    /// Whether a request may proceed. Transitions open -> half_open once the
    /// cooldown elapsed, allowing exactly one probe request.
    pub fn allow(&self) -> bool {
        let mut guard = self.inner.write().expect("breaker lock poisoned");
        match guard.state {
            State::Closed => true,
            State::HalfOpen => !guard.probe_pending,
            State::Open => {
                let elapsed = guard
                    .opened_at
                    .map(|t| t.elapsed())
                    .unwrap_or_else(|| self.cooldown + Duration::from_secs(1));
                if elapsed >= self.cooldown {
                    guard.state = State::HalfOpen;
                    guard.probe_pending = true;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn record_success(&self) {
        let mut guard = self.inner.write().expect("breaker lock poisoned");
        guard.state = State::Closed;
        guard.consecutive_failures = 0;
        guard.probe_pending = false;
        guard.opened_at = None;
    }

    pub fn record_failure(&self) {
        let mut guard = self.inner.write().expect("breaker lock poisoned");
        guard.consecutive_failures = guard.consecutive_failures.saturating_add(1);
        match guard.state {
            State::HalfOpen => {
                // Probe failed: back to open immediately.
                guard.state = State::Open;
                guard.opened_at = Some(Instant::now());
                guard.probe_pending = false;
            }
            State::Closed => {
                if guard.consecutive_failures >= self.threshold {
                    guard.state = State::Open;
                    guard.opened_at = Some(Instant::now());
                }
            }
            State::Open => {
                guard.probe_pending = false;
            }
        }
    }
}

/// Registry of per-account breakers.
#[derive(Debug, Default)]
pub struct CircuitRegistry {
    breakers: DashMap<String, std::sync::Arc<Breaker>>,
}

impl CircuitRegistry {
    pub fn new() -> Self {
        Self {
            breakers: DashMap::new(),
        }
    }

    pub fn get_or_create(
        &self,
        account_id: &str,
        threshold: u32,
        cooldown: Duration,
    ) -> std::sync::Arc<Breaker> {
        self.breakers
            .entry(account_id.to_string())
            .or_insert_with(|| std::sync::Arc::new(Breaker::new(threshold, cooldown)))
            .clone()
    }

    pub fn remove(&self, account_id: &str) {
        self.breakers.remove(account_id);
    }

    pub fn count(&self) -> usize {
        self.breakers.len()
    }
}
