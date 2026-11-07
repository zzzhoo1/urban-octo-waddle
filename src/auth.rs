use std::collections::HashMap;
use std::sync::Arc;

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, SaltString};
use argon2::{Argon2, PasswordVerifier};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use crate::config::QuotaConfig;

/// A configured API key with quota and model restrictions.
#[derive(Debug, Clone)]
pub struct ApiKey {
    pub name: String,
    /// argon2id PHC string (new) or sha256 hex (legacy, lazily upgraded).
    pub hash: String,
    pub role: Role,
    pub allowed_models: Vec<String>,
    pub token_quota: u64,
    pub tokens_used: u64,
    pub tokens_used_in_period: u64,
    pub quota_period: QuotaPeriod,
    pub quota_period_start: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    /// Deterministic fingerprint used as the accounting index (never the
    /// plaintext digest).
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Admin,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Role::User),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaPeriod {
    Total,
    Daily,
    Monthly,
}

impl QuotaPeriod {
    pub fn as_str(&self) -> &'static str {
        match self {
            QuotaPeriod::Total => "total",
            QuotaPeriod::Daily => "daily",
            QuotaPeriod::Monthly => "monthly",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "total" => Some(QuotaPeriod::Total),
            "daily" => Some(QuotaPeriod::Daily),
            "monthly" => Some(QuotaPeriod::Monthly),
            _ => None,
        }
    }
}

/// Authentication and quota accounting for API keys.
///
/// Keys live in `keys.json`; usage counters live in SQLite (`usage.db`) as
/// of issue #3 suggestion 4. The in-memory map is the hot path for quota
/// checks; the background persist loop flushes changed counters to SQLite
/// in batches, and structural changes (lazy hash upgrade, admin edits) to
/// `keys.json`.
pub struct KeyAuth {
    keys: RwLock<HashMap<String, ApiKey>>,
    /// Fingerprints whose usage counters changed since the last flush
    /// (needs a SQLite usage upsert).
    usage_dirty: RwLock<Vec<String>>,
    /// Fingerprints with structural changes (lazy hash upgrade) that need a
    /// keys.json rewrite.
    dirty: RwLock<Vec<String>>,
}

impl KeyAuth {
    pub fn new() -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
            usage_dirty: RwLock::new(Vec::new()),
            dirty: RwLock::new(Vec::new()),
        }
    }

    /// Load keys from `keys.json`; missing file -> empty map.
    pub fn load(path: &str, admin_key: &str) -> anyhow::Result<Self> {
        let mut this = Self::new();
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let keys: Vec<ApiKey> = serde_json::from_str(&content)
                    .map_err(|e| anyhow::anyhow!("failed to parse {}: {}", path, e))?;
                let mut map = HashMap::new();
                for k in keys {
                    map.insert(k.fingerprint.clone(), k);
                }
                this.keys = RwLock::new(map);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // First run: create the default admin key.
                let admin = ApiKey {
                    name: "admin".to_string(),
                    hash: hash_api_key(admin_key),
                    role: Role::Admin,
                    allowed_models: Vec::new(),
                    token_quota: 0,
                    tokens_used: 0,
                    tokens_used_in_period: 0,
                    quota_period: QuotaPeriod::Total,
                    quota_period_start: None,
                    expires_at: None,
                    enabled: true,
                    fingerprint: fingerprint_of_key(admin_key),
                };
                let mut map = HashMap::new();
                map.insert(admin.fingerprint.clone(), admin);
                this.keys = RwLock::new(map);
                this.dirty.write().push(admin.fingerprint.clone());
            }
            Err(e) => return Err(anyhow::anyhow!("failed to read {}: {}", path, e)),
        }
        Ok(this)
    }

    /// Deterministic fingerprint for an API key (never the plaintext digest).
    pub fn fingerprint_of(&self, api_key: &str) -> String {
        fingerprint_of_key(api_key)
    }

    /// Authenticate a key for a specific model (or empty for any).
    pub fn authenticate_for_model(
        &self,
        api_key: &str,
        model: &str,
        resolved_model: &str,
    ) -> Result<ApiKey, AuthFailure> {
        let fp = fingerprint_of_key(api_key);
        let guard = self.keys.read().map_err(|_| AuthFailure::UnknownKey)?;
        let key = guard.get(&fp).ok_or(AuthFailure::UnknownKey)?;
        if !key.enabled {
            return Err(AuthFailure::Disabled);
        }
        if let Some(exp) = key.expires_at {
            if Utc::now() > exp {
                return Err(AuthFailure::Expired);
            }
        }
        // Model allowlist: empty means all models allowed.
        if !key.allowed_models.is_empty() {
            let allowed = key
                .allowed_models
                .iter()
                .any(|m| m == model || m == resolved_model);
            if !allowed {
                return Err(AuthFailure::ModelNotAllowed);
            }
        }
        // Quota check (soft threshold at 90% is only a warning).
        if self.quota_exceeded(&fp) {
            return Err(AuthFailure::QuotaExceeded);
        }
        Ok(key.clone())
    }

    /// Authenticate a key for any model (used by /v1/models etc.).
    pub fn authenticate(&self, api_key: &str) -> bool {
        let fp = fingerprint_of_key(api_key);
        let guard = match self.keys.read() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let Some(key) = guard.get(&fp) else {
            return false;
        };
        key.enabled
    }

    /// A2: lazily upgrade a legacy sha256 entry to argon2id after a
    /// successful authentication. Failure does not affect the request.
    pub fn lazy_upgrade(&self, api_key: &str) {
        let fp = fingerprint_of_key(api_key);
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), lazy upgrade dropped");
                return;
            }
        };
        let Some(key) = guard.get_mut(&fp) else {
            return;
        };
        if key.hash.starts_with("$argon2") {
            return; // already argon2id
        }
        let salt = SaltString::generate(&mut OsRng);
        match Argon2::default().hash_password(api_key.as_bytes(), &salt) {
            Ok(hash) => {
                key.hash = hash.to_string();
                if !self.dirty.read().contains(&fp) {
                    self.dirty.write().push(fp);
                }
            }
            Err(e) => {
                tracing::warn!("lazy argon2 upgrade failed: {}", e);
            }
        }
    }

    /// B3: roll quota period counters when the period boundary is crossed.
    pub fn roll_quota_periods(&self) {
        let now = Utc::now();
        let mut usage_dirty = Vec::new();
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), quota roll skipped");
                return;
            }
        };
        for k in guard.values_mut() {
            if k.quota_period == QuotaPeriod::Total {
                continue;
            }
            let boundary = match k.quota_period {
                QuotaPeriod::Daily => daily_boundary(now),
                QuotaPeriod::Monthly => monthly_boundary(now),
                QuotaPeriod::Total => continue,
            };
            let needs_roll = match k.quota_period_start {
                Some(start) => start < boundary,
                None => true,
            };
            if needs_roll {
                k.tokens_used_in_period = 0;
                k.quota_period_start = Some(boundary);
                if !usage_dirty.contains(&k.fingerprint) {
                    usage_dirty.push(k.fingerprint.clone());
                }
            }
        }
        if !usage_dirty.is_empty() {
            self.restore_usage_dirty(usage_dirty);
        }
    }

    /// B3: quota snapshot for response headers and soft-threshold warning.
    pub fn quota_snapshot(&self, fingerprint: &str) -> (u64, u64, bool) {
        let guard = match self.keys.read() {
            Ok(g) => g,
            Err(_) => return (0, 0, false),
        };
        let Some(key) = guard.get(fingerprint) else {
            return (0, 0, false);
        };
        let used = key.tokens_used;
        let quota = key.token_quota;
        let warning = quota > 0 && used * 10 >= quota * 9;
        (used, quota, warning)
    }

    /// B3: hard quota check (used in authenticate_for_model).
    fn quota_exceeded(&self, fingerprint: &str) -> bool {
        let guard = match self.keys.read() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let Some(key) = guard.get(fingerprint) else {
            return false;
        };
        if key.token_quota == 0 {
            return false; // unlimited
        }
        key.tokens_used >= key.token_quota
    }

    /// Record token usage against a key after a completed request.
    pub fn record_usage(&self, fingerprint: &str, tokens: u64) {
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), usage not recorded");
                return;
            }
        };
        let Some(key) = guard.get_mut(fingerprint) else {
            return;
        };
        key.tokens_used += tokens;
        key.tokens_used_in_period += tokens;
        if tokens > 0 {
            let mut usage_dirty = self.usage_dirty.write().map_err(|_| ()).ok();
            if let Some(d) = usage_dirty.as_mut() {
                if !d.contains(fingerprint) {
                    d.push(fingerprint.to_string());
                }
            }
        }
    }

    /// List all keys (for admin listing and persistence).
    pub fn list(&self) -> Vec<ApiKey> {
        let guard = match self.keys.read() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        guard.values().cloned().collect()
    }

    /// Add or replace a key (admin operation).
    pub fn upsert_key(&self, key: ApiKey) {
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), key upsert dropped");
                return;
            }
        };
        guard.insert(key.fingerprint.clone(), key);
    }

    /// Remove a key by fingerprint (admin operation).
    pub fn remove_key(&self, fingerprint: &str) -> bool {
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), key remove dropped");
                return false;
            }
        };
        guard.remove(fingerprint).is_some()
    }

    /// Drain fingerprints whose structural state changed since the last
    /// call, so the caller can persist them to keys.json.
    pub fn take_dirty(&self) -> Vec<String> {
        match self.dirty.write() {
            Ok(mut d) => std::mem::take(&mut *d),
            Err(_) => {
                tracing::error!("auth keys lock poisoned (read)");
                Vec::new()
            }
        }
    }

    /// Put fingerprints back after a failed keys.json persist so the next
    /// cycle retries. Entries already pending are not duplicated.
    pub fn restore_dirty(&self, fps: Vec<String>) {
        if fps.is_empty() {
            return;
        }
        match self.dirty.write() {
            Ok(mut d) => {
                for fp in fps {
                    if !d.contains(&fp) {
                        d.push(fp);
                    }
                }
            }
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), restore skipped");
            }
        }
    }

    /// Drain fingerprints whose usage counters changed since the last call,
    /// so the caller can persist them to the usage store.
    pub fn take_usage_dirty(&self) -> Vec<String> {
        match self.usage_dirty.write() {
            Ok(mut d) => std::mem::take(&mut *d),
            Err(_) => {
                tracing::error!("usage dirty lock poisoned (read)");
                Vec::new()
            }
        }
    }

    /// Put fingerprints back after a failed usage persist so the next cycle
    /// retries. Entries already pending are not duplicated.
    pub fn restore_usage_dirty(&self, fps: Vec<String>) {
        if fps.is_empty() {
            return;
        }
        match self.usage_dirty.write() {
            Ok(mut d) => {
                for fp in fps {
                    if !d.contains(&fp) {
                        d.push(fp);
                    }
                }
            }
            Err(_) => {
                tracing::error!("usage dirty lock poisoned (write), restore skipped");
            }
        }
    }

    /// Current in-memory usage counters for one key, used by the background
    /// flush to write absolute values into the usage store.
    pub fn usage_snapshot(
        &self,
        fingerprint: &str,
    ) -> Option<(u64, u64, Option<DateTime<Utc>>)> {
        let guard = self.keys.read().ok()?;
        let k = guard.get(fingerprint)?;
        Some((
            k.tokens_used,
            k.tokens_used_in_period,
            k.quota_period_start,
        ))
    }
}

/// Errors returned by authenticate_for_model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    UnknownKey,
    Expired,
    Disabled,
    QuotaExceeded,
    ModelNotAllowed,
    Forbidden,
}

/// Hash an API key with argon2id (new keys).
pub fn hash_api_key(key: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(key.as_bytes(), &salt)
        .expect("argon2 hash failed")
        .to_string()
}

/// Verify an API key against a stored hash (argon2id or legacy sha256).
pub fn verify_api_key(key: &str, hash: &str) -> bool {
    if hash.starts_with("$argon2") {
        match PasswordHash::new(hash) {
            Ok(parsed) => Argon2::default()
                .verify_password(key.as_bytes(), &parsed)
                .is_ok(),
            Err(_) => false,
        }
    } else {
        // Legacy sha256 hex.
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let digest = hasher.finalize();
        B64.encode(digest) == hash || hex::encode(digest) == hash
    }
}

/// Deterministic fingerprint for an API key (never the plaintext digest).
pub fn fingerprint_of_key(api_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    let digest = hasher.finalize();
    B64.encode(digest)
}

/// Start of the current UTC day.
fn daily_boundary(now: DateTime<Utc>) -> DateTime<Utc> {
    let secs = now.timestamp();
    let day_start = secs - (secs % 86400);
    DateTime::from_timestamp(day_start, 0).unwrap_or(now)
}

/// Start of the current UTC month.
fn monthly_boundary(now: DateTime<Utc>) -> DateTime<Utc> {
    let (year, month, _day) = (now.year(), now.month(), now.day());
    DateTime::from_timestamp(
        chrono::NaiveDate::from_ymd_opt(year, month, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp(),
        0,
    )
    .unwrap_or(now)
}
