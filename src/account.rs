use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Account lifecycle status, serialized exactly like the legacy gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountStatus {
    Active,
    Disabled,
    Cooling,
}

impl Default for AccountStatus {
    fn default() -> Self {
        AccountStatus::Active
    }
}

impl AccountStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountStatus::Active => "active",
            AccountStatus::Disabled => "disabled",
            AccountStatus::Cooling => "cooling",
        }
    }
}

impl std::fmt::Display for AccountStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Persisted account snapshot. `credential` is always stored encrypted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountData {
    pub id: String,
    pub name: String,
    pub credential: String,
    #[serde(default)]
    pub status: AccountStatus,
    #[serde(default)]
    pub failure_count: u32,
    #[serde(default)]
    pub last_failure_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Runtime account guarded by a lock so the pool can update it concurrently.
#[derive(Debug)]
pub struct Account {
    inner: RwLock<AccountData>,
}

impl Account {
    pub fn new(id: String, name: String, credential: String) -> Self {
        Self {
            inner: RwLock::new(AccountData {
                id,
                name,
                credential,
                status: AccountStatus::Active,
                failure_count: 0,
                last_failure_at: None,
                created_at: Utc::now(),
            }),
        }
    }

    pub fn from_data(data: AccountData) -> Self {
        Self {
            inner: RwLock::new(data),
        }
    }

    pub fn data(&self) -> AccountData {
        match self.inner.read() {
            Ok(g) => g.clone(),
            Err(_) => {
                tracing::error!("account lock poisoned (read)");
                AccountData {
                    id: String::new(),
                    name: String::new(),
                    credential: String::new(),
                    status: AccountStatus::Disabled,
                    failure_count: 0,
                    last_failure_at: None,
                    created_at: Utc::now(),
                }
            }
        }
    }

    pub fn id(&self) -> String {
        match self.inner.read() {
            Ok(g) => g.id.clone(),
            Err(_) => {
                tracing::error!("account lock poisoned (read)");
                String::new()
            }
        }
    }

    pub fn name(&self) -> String {
        match self.inner.read() {
            Ok(g) => g.name.clone(),
            Err(_) => {
                tracing::error!("account lock poisoned (read)");
                String::new()
            }
        }
    }

    pub fn status(&self) -> AccountStatus {
        match self.inner.read() {
            Ok(g) => g.status,
            Err(_) => {
                tracing::error!("account lock poisoned (read)");
                AccountStatus::Disabled
            }
        }
    }

    pub fn failure_count(&self) -> u32 {
        match self.inner.read() {
            Ok(g) => g.failure_count,
            Err(_) => {
                tracing::error!("account lock poisoned (read)");
                0
            }
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.status() == AccountStatus::Active
    }

    pub fn enable(&self) {
        match self.inner.write() {
            Ok(mut guard) => {
                guard.status = AccountStatus::Active;
                guard.failure_count = 0;
            }
            Err(_) => {
                tracing::error!("account lock poisoned (write), enable dropped");
            }
        }
    }

    pub fn disable(&self) {
        match self.inner.write() {
            Ok(mut guard) => {
                guard.status = AccountStatus::Disabled;
            }
            Err(_) => {
                tracing::error!("account lock poisoned (write), disable dropped");
            }
        }
    }

    /// Record a failure. Returns true when the account transitions to cooling.
    pub fn mark_failure(&self, now: DateTime<Utc>, threshold: u32) -> bool {
        let mut guard = match self.inner.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("account lock poisoned (write), mark_failure dropped");
                return false;
            }
        };
        guard.failure_count = guard.failure_count.saturating_add(1);
        guard.last_failure_at = Some(now);
        if threshold == 0 || guard.failure_count >= threshold {
            if guard.status != AccountStatus::Cooling {
                guard.status = AccountStatus::Cooling;
                return true;
            }
        }
        false
    }

    pub fn reset_failures(&self) {
        match self.inner.write() {
            Ok(mut guard) => {
                guard.failure_count = 0;
            }
            Err(_) => {
                tracing::error!("account lock poisoned (write), reset dropped");
            }
        }
    }

    /// Recover a cooling account once the cooldown elapsed.
    pub fn recover_if_due(&self, now: DateTime<Utc>, recover_after: Duration) -> bool {
        if recover_after.is_zero() {
            return false;
        }
        let mut guard = match self.inner.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("account lock poisoned (write), recover dropped");
                return false;
            }
        };
        if guard.status != AccountStatus::Cooling {
            return false;
        }
        let Some(last) = guard.last_failure_at else {
            return false;
        };
        let elapsed = now
            .signed_duration_since(last)
            .to_std()
            .unwrap_or_else(|_| Duration::ZERO);
        if elapsed >= recover_after {
            guard.status = AccountStatus::Active;
            guard.failure_count = 0;
            return true;
        }
        false
    }
}

/// In-memory account pool with health-subset round-robin selection.
#[derive(Debug, Default)]
pub struct Pool {
    accounts: RwLock<Vec<Arc<Account>>>,
    cursor: AtomicUsize,
}

impl Pool {
    pub fn new() -> Self {
        Self {
            accounts: RwLock::new(Vec::new()),
            cursor: AtomicUsize::new(0),
        }
    }

    pub fn add(&self, account: Account) {
        match self.accounts.write() {
            Ok(mut guard) => guard.push(Arc::new(account)),
            Err(_) => {
                tracing::error!("pool lock poisoned (write), account add dropped");
            }
        }
    }

    pub fn add_data(&self, data: AccountData) {
        self.add(Account::from_data(data));
    }

    pub fn list(&self) -> Vec<Arc<Account>> {
        match self.accounts.read() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                tracing::error!("pool lock poisoned (read)");
                Vec::new()
            }
        }
    }

    pub fn list_data(&self) -> Vec<AccountData> {
        self.list().iter().map(|account| account.data()).collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Account>> {
        match self.accounts.read() {
            Ok(guard) => guard.iter().find(|account| account.id() == id).cloned(),
            Err(_) => {
                tracing::error!("pool lock poisoned (read)");
                None
            }
        }
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut guard = match self.accounts.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("pool lock poisoned (write), remove dropped");
                return false;
            }
        };
        let Some(index) = guard.iter().position(|account| account.id() == id) else {
            return false;
        };
        guard.remove(index);
        true
    }

    pub fn recover_cooling(&self, now: DateTime<Utc>, recover_after: Duration) -> usize {
        self.list()
            .iter()
            .filter(|account| account.recover_if_due(now, recover_after))
            .count()
    }

    /// Pick the next healthy account. Cooling accounts due for recovery are
    /// restored first, mirroring the legacy gateway behaviour.
    pub fn select(&self, now: DateTime<Utc>, recover_after: Duration) -> Option<Arc<Account>> {
        let _ = self.recover_cooling(now, recover_after);
        let healthy: Vec<Arc<Account>> = self
            .list()
            .into_iter()
            .filter(|account| account.is_healthy())
            .collect();
        if healthy.is_empty() {
            return None;
        }
        let start = self.cursor.fetch_add(1, Ordering::Relaxed);
        Some(healthy[start % healthy.len()].clone())
    }
}
