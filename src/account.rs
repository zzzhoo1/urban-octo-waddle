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
        self.inner.read().expect("account lock poisoned").clone()
    }

    pub fn id(&self) -> String {
        self.inner.read().expect("account lock poisoned").id.clone()
    }

    pub fn name(&self) -> String {
        self.inner.read().expect("account lock poisoned").name.clone()
    }

    pub fn status(&self) -> AccountStatus {
        self.inner.read().expect("account lock poisoned").status
    }

    pub fn failure_count(&self) -> u32 {
        self.inner
            .read()
            .expect("account lock poisoned")
            .failure_count
    }

    pub fn is_healthy(&self) -> bool {
        self.status() == AccountStatus::Active
    }

    pub fn enable(&self) {
        let mut guard = self.inner.write().expect("account lock poisoned");
        guard.status = AccountStatus::Active;
        guard.failure_count = 0;
    }

    pub fn disable(&self) {
        let mut guard = self.inner.write().expect("account lock poisoned");
        guard.status = AccountStatus::Disabled;
    }

    /// Record a failure. Returns true when the account transitions to cooling.
    pub fn mark_failure(&self, now: DateTime<Utc>, threshold: u32) -> bool {
        let mut guard = self.inner.write().expect("account lock poisoned");
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
        let mut guard = self.inner.write().expect("account lock poisoned");
        guard.failure_count = 0;
    }

    /// Recover a cooling account once the cooldown elapsed.
    pub fn recover_if_due(&self, now: DateTime<Utc>, recover_after: Duration) -> bool {
        if recover_after.is_zero() {
            return false;
        }
        let mut guard = self.inner.write().expect("account lock poisoned");
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
        self.accounts
            .write()
            .expect("pool lock poisoned")
            .push(Arc::new(account));
    }

    pub fn add_data(&self, data: AccountData) {
        self.add(Account::from_data(data));
    }

    pub fn list(&self) -> Vec<Arc<Account>> {
        self.accounts.read().expect("pool lock poisoned").clone()
    }

    pub fn list_data(&self) -> Vec<AccountData> {
        self.list().iter().map(|account| account.data()).collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Account>> {
        self.accounts
            .read()
            .expect("pool lock poisoned")
            .iter()
            .find(|account| account.id() == id)
            .cloned()
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut guard = self.accounts.write().expect("pool lock poisoned");
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
