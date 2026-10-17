use std::collections::HashMap;
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Role of an API key. Legacy entries without a role field are treated as
/// admin so existing keys.json files keep working unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyRole {
    Admin,
    User,
}

impl Default for KeyRole {
    fn default() -> Self {
        // A2 兼容规则：缺失角色字段的旧条目一律视为 admin。
        KeyRole::Admin
    }
}

impl KeyRole {
    pub fn as_str(self) -> &'static str {
        match self {
            KeyRole::Admin => "admin",
            KeyRole::User => "user",
        }
    }
}

impl std::fmt::Display for KeyRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn default_role() -> KeyRole {
    KeyRole::Admin
}

/// Deserialize a key role: missing/null -> admin (legacy compat), unknown
/// string -> user with a WARN (A2 spec: 未知角色取值按 user 处理)。
fn deserialize_role<'de, D>(d: D) -> Result<KeyRole, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(d)?;
    Ok(match raw.as_deref() {
        None | Some("admin") => KeyRole::Admin,
        Some("user") => KeyRole::User,
        Some(other) => {
            tracing::warn!(role = %other, "unknown key role in keys.json, treated as user");
            KeyRole::User
        }
    })
}

fn default_true() -> bool {
    true
}

/// Quota reset period (B3). `none` keeps the legacy cumulative semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuotaPeriod {
    None,
    Daily,
    Monthly,
}

impl Default for QuotaPeriod {
    fn default() -> Self {
        QuotaPeriod::None
    }
}

impl QuotaPeriod {
    pub fn as_str(self) -> &'static str {
        match self {
            QuotaPeriod::None => "none",
            QuotaPeriod::Daily => "daily",
            QuotaPeriod::Monthly => "monthly",
        }
    }

    /// Parse a period from a string (B3: unknown values are rejected by the
    /// admin API with 400; here they map to None for serde leniency).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(QuotaPeriod::None),
            "daily" => Some(QuotaPeriod::Daily),
            "monthly" => Some(QuotaPeriod::Monthly),
            _ => None,
        }
    }

    /// Start of the period containing `now` (UTC).
    pub fn period_start(self, now: DateTime<Utc>) -> DateTime<Utc> {
        use chrono::Datelike;
        match self {
            QuotaPeriod::None => DateTime::from_timestamp(0, 0).unwrap_or(now),
            QuotaPeriod::Daily => {
                DateTime::from_timestamp(now.timestamp() - (now.timestamp() % 86_400), 0)
                    .unwrap_or(now)
            }
            QuotaPeriod::Monthly => now
                .with_day(1)
                .and_then(|d| d.with_hour(0))
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap_or(now),
        }
    }
}

/// A named API key with optional expiry, per-model allowlist and token quota.
/// The plaintext key is stored hashed so the file can be backed up without
/// exposing usable secrets. New keys use argon2id with a per-key random salt;
/// legacy entries keep the unsalted SHA-256 digest until they are lazily
/// upgraded on the next successful authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    /// Display name shown in logs and the panel.
    pub name: String,
    /// Digest of the plaintext key: argon2id PHC string for new keys,
    /// legacy SHA-256 hex for entries not yet rotated.
    pub key_hash: String,
    /// Deterministic lookup fingerprint (HMAC-SHA256 of the plaintext under
    /// the server secret). Missing for legacy entries, computed on load.
    #[serde(default)]
    pub fingerprint: String,
    /// Masked hint for the panel, e.g. "sk-...9f3c".
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    /// Max total tokens this key may consume (0 = unlimited).
    #[serde(default)]
    pub token_quota: u64,
    /// Tokens consumed so far (persisted).
    #[serde(default)]
    pub tokens_used: u64,
    /// Allowed upstream model ids; empty = all models allowed.
    #[serde(default)]
    pub model_allowlist: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Admin or data-plane-only role. Missing in legacy data = admin;
    /// unknown values are treated as user with a WARN (A2).
    #[serde(default = "default_role", deserialize_with = "deserialize_role")]
    pub role: KeyRole,
    /// True for the key injected via --gw-key; cannot be modified or deleted.
    #[serde(default)]
    pub is_master: bool,
    /// True while the entry still uses the legacy unsalted SHA-256 digest.
    #[serde(default)]
    pub legacy_hash: bool,
    /// B3: quota reset period (none/daily/monthly), default none (cumulative).
    #[serde(default)]
    pub quota_period: QuotaPeriod,
    /// B3: start of the current quota period (UTC); 0 for period=none.
    #[serde(default)]
    pub quota_period_start: Option<DateTime<Utc>>,
    /// B3: tokens consumed within the current period.
    #[serde(default)]
    pub tokens_used_in_period: u64,
}

impl ApiKey {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.map(|t| now > t).unwrap_or(false)
    }

    /// B3: lazily roll the quota period forward if the boundary has passed.
    /// Returns true when a roll happened (caller should persist).
    pub fn roll_quota_period(&mut self, now: DateTime<Utc>) -> bool {
        if self.quota_period == QuotaPeriod::None {
            return false;
        }
        let start = self.quota_period_start.unwrap_or(now);
        let cur = self.quota_period.period_start(now);
        if start < cur {
            self.quota_period_start = Some(cur);
            self.tokens_used_in_period = 0;
            return true;
        }
        false
    }

    /// B3: quota check based on in-period usage (falls back to cumulative
    /// usage when the period is none, preserving legacy semantics).
    pub fn quota_exceeded(&self) -> bool {
        if self.token_quota == 0 {
            return false;
        }
        if self.quota_period == QuotaPeriod::None {
            self.tokens_used >= self.token_quota
        } else {
            self.tokens_used_in_period >= self.token_quota
        }
    }

    /// B3: in-period usage ratio (0.0-1.0+); None when unlimited.
    pub fn quota_usage_ratio(&self) -> Option<f64> {
        if self.token_quota == 0 {
            return None;
        }
        let used = if self.quota_period == QuotaPeriod::None {
            self.tokens_used
        } else {
            self.tokens_used_in_period
        };
        Some(used as f64 / self.token_quota as f64)
    }

    pub fn model_allowed(&self, model: &str) -> bool {
        self.model_allowlist.is_empty() || self.model_allowlist.iter().any(|m| m == model)
    }
}

/// Legacy unsalted SHA-256 hex digest (kept for reading old keys.json).
pub fn hash_key(plaintext: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

pub fn mask_key(plaintext: &str) -> String {
    // A1: 按字符而非字节切片，避免多字节字符在边界处 panic。
    let chars: Vec<char> = plaintext.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let prefix: String = chars[..4].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{}...{}", prefix, suffix)
}

/// Deterministic lookup fingerprint: HMAC-SHA256 of the plaintext keyed by
/// the server secret. Unlike the slow argon2id digest this can be recomputed
/// cheaply at request time and used as the HashMap key, while remaining
/// useless to anyone who obtains keys.json without the server secret.
pub fn fingerprint_key(secret: &str, plaintext: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    // HMAC accepts keys of any length, so this cannot fail.
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(b"key-fingerprint:");
    mac.update(plaintext.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Hash a plaintext key with argon2id (OWASP baseline parameters) and a
/// fresh random salt. Returns the PHC string.
pub fn hash_key_slow(plaintext: &str) -> String {
    use argon2::password_hash::{PasswordHasher, SaltString};
    use rand::RngCore;
    let mut salt_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt_bytes);
    let Ok(salt) = SaltString::encode_b64(&salt_bytes) else {
        return String::new();
    };
    argon2::Argon2::default()
        .hash_password(plaintext.as_bytes(), &salt)
        .map(|h| h.to_string())
        .unwrap_or_else(|_| String::new())
}

/// Verify a plaintext key against an argon2id PHC string.
pub fn verify_key_slow(plaintext: &str, phc: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    argon2::Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .is_ok()
}

/// Result of a successful authentication.
#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub key_name: String,
    pub role: KeyRole,
    pub is_master: bool,
}

/// Authentication errors surfaced to callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    UnknownKey,
    Expired,
    Disabled,
    QuotaExceeded,
    ModelNotAllowed,
    /// Authenticated key is valid but lacks admin privilege (A2).
    Forbidden,
}

/// Multi-key authenticator shared by the data plane (/v1) and admin plane.
/// Keys persist to a JSON file. Admin-plane access requires the `admin`
/// role; data-plane access ignores the role (A2).
pub struct KeyAuth {
    keys: RwLock<HashMap<String, ApiKey>>, // fingerprint -> ApiKey
    /// Server secret used to derive lookup fingerprints.
    secret: String,
    /// Lazily-upgraded entries waiting to be persisted (by fingerprint).
    dirty: RwLock<Vec<String>>,
}

impl KeyAuth {
    pub fn new(secret: &str, master_plaintext: Vec<String>) -> Self {
        let mut map = HashMap::new();
        for (i, k) in master_plaintext.into_iter().filter(|k| !k.is_empty()).enumerate() {
            let name = if i == 0 { "master".to_string() } else { format!("key-{}", i) };
            let fp = fingerprint_key(secret, &k);
            map.entry(fp.clone()).or_insert(ApiKey {
                name,
                key_hash: hash_key_slow(&k),
                fingerprint: fp,
                hint: mask_key(&k),
                created_at: Some(Utc::now()),
                expires_at: None,
                token_quota: 0,
                tokens_used: 0,
                model_allowlist: Vec::new(),
                enabled: true,
                role: KeyRole::Admin,
                is_master: i == 0,
                legacy_hash: false,
            });
        }
        Self {
            keys: RwLock::new(map),
            secret: secret.to_string(),
            dirty: RwLock::new(Vec::new()),
        }
    }

    pub fn from_entries(secret: &str, entries: Vec<ApiKey>) -> Self {
        let mut map = HashMap::new();
        for mut k in entries.into_iter() {
            // Legacy entries predate the fingerprint field; we cannot derive
            // it without the plaintext, so they are indexed by their SHA-256
            // digest and get a real fingerprint on lazy upgrade. A2 兼容：
            // 未知角色取值按 user 处理并记录 WARN（serde 无法解析时默认值
            // 兜底为 admin，因此这里仅处理显式的非 admin/user 字符串情形）。
            if k.fingerprint.is_empty() {
                k.legacy_hash = true;
                k.fingerprint = format!("sha256:{}", k.key_hash);
            }
            map.insert(k.fingerprint.clone(), k);
        }
        Self {
            keys: RwLock::new(map),
            secret: secret.to_string(),
            dirty: RwLock::new(Vec::new()),
        }
    }

    fn lookup(&self, plaintext: &str) -> Option<ApiKey> {
        if plaintext.is_empty() {
            return None;
        }
        let fp = fingerprint_key(&self.secret, plaintext);
        let guard = self.keys.read().ok()?;
        if let Some(k) = guard.get(&fp) {
            // New-format entry: verify against the slow digest as well.
            if !k.legacy_hash && !verify_key_slow(plaintext, &k.key_hash) {
                return None;
            }
            return Some(k.clone());
        }
        // Legacy path: indexed by unsalted SHA-256 digest.
        let legacy_fp = format!("sha256:{}", hash_key(plaintext));
        guard.get(&legacy_fp).cloned()
    }

    /// Lazily upgrade a legacy SHA-256 entry to argon2id + HMAC fingerprint
    /// after a successful authentication. Never fails the request; on any
    /// error the entry is left untouched and the upgrade is retried later.
    pub fn lazy_upgrade(&self, plaintext: &str) {
        let fp = fingerprint_key(&self.secret, plaintext);
        let legacy_fp = format!("sha256:{}", hash_key(plaintext));
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), lazy upgrade dropped");
                return;
            }
        };
        let needs = guard
            .get(&legacy_fp)
            .map(|k| k.legacy_hash)
            .unwrap_or(false);
        if !needs {
            return;
        }
        let Some(mut entry) = guard.remove(&legacy_fp) else {
            return;
        };
        let new_hash = hash_key_slow(plaintext);
        if new_hash.is_empty() {
            guard.insert(legacy_fp, entry);
            tracing::error!("argon2id hashing failed during lazy upgrade, will retry");
            return;
        }
        entry.key_hash = new_hash;
        entry.fingerprint = fp.clone();
        entry.legacy_hash = false;
        guard.insert(fp.clone(), entry);
        drop(guard);
        match self.dirty.write() {
            Ok(mut d) => d.push(fp),
            Err(_) => {
                tracing::error!("auth dirty lock poisoned (write), upgrade persist skipped");
            }
        }
        tracing::info!("api key hash lazily upgraded to argon2id");
    }

    /// Drain the list of fingerprints whose entries were lazily upgraded
    /// since the last call, so the caller can persist them.
    pub fn take_dirty(&self) -> Vec<String> {
        match self.dirty.write() {
            Ok(mut d) => std::mem::take(&mut *d),
            Err(_) => {
                tracing::error!("auth dirty lock poisoned (read)");
                Vec::new()
            }
        }
    }

    /// Authenticate a data-plane caller for a specific model. The role does
    /// not participate in data-plane decisions (A2 规格第 3 条)。
    /// B1: allowlist matches the client-supplied name OR the resolved
    /// upstream model name — either hit passes.
    pub fn authenticate_for_model(
        &self,
        plaintext: &str,
        model: &str,
        resolved_model: &str,
    ) -> Result<AuthIdentity, AuthFailure> {
        let Some(key) = self.lookup(plaintext) else {
            return Err(AuthFailure::UnknownKey);
        };
        if !key.enabled {
            return Err(AuthFailure::Disabled);
        }
        if key.is_expired(Utc::now()) {
            return Err(AuthFailure::Expired);
        }
        if key.quota_exceeded() {
            return Err(AuthFailure::QuotaExceeded);
        }
        let client_ok = model.is_empty() || key.model_allowed(model);
        let resolved_ok = !resolved_model.is_empty() && key.model_allowed(resolved_model);
        if !client_ok && !resolved_ok {
            return Err(AuthFailure::ModelNotAllowed);
        }
        Ok(AuthIdentity {
            key_name: key.name.clone(),
            role: key.role,
            is_master: key.is_master,
        })
    }

    /// B3: quota snapshot for X-Gateway-* headers: (in-period usage,
    /// quota, soft-threshold warning at 90%).
    pub fn quota_snapshot(&self, fingerprint: &str) -> (u64, u64, bool) {
        match self.keys.read() {
            Ok(g) => match g.get(fingerprint) {
                Some(k) => {
                    let used = if k.quota_period == QuotaPeriod::None {
                        k.tokens_used
                    } else {
                        k.tokens_used_in_period
                    };
                    let warning = k.token_quota > 0 && used * 10 >= k.token_quota * 9;
                    (used, k.token_quota, warning)
                }
                None => (0, 0, false),
            },
            Err(_) => {
                tracing::error!("auth keys lock poisoned (read)");
                (0, 0, false)
            }
        }
    }

    /// B3: lazily roll quota periods at authentication time. Rolled entries
    /// are marked dirty so the existing keys.json persist path flushes them.
    pub fn roll_quota_periods(&self) {
        let now = Utc::now();
        let mut rolled = Vec::new();
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), quota roll dropped");
                return;
            }
        };
        for (fp, key) in guard.iter_mut() {
            if key.roll_quota_period(now) {
                rolled.push(fp.clone());
            }
        }
        drop(guard);
        if !rolled.is_empty() {
            match self.dirty.write() {
                Ok(mut d) => d.extend(rolled.iter().cloned()),
                Err(_) => {
                    tracing::error!("auth dirty lock poisoned (write), quota roll persist skipped");
                }
            }
            tracing::info!(count = rolled.len(), "quota periods rolled");
        }
    }

    /// Admin-plane auth: valid key + admin role required. Returns
    /// Err(Forbidden) for valid non-admin keys so the caller can answer 403
    /// instead of 401 (A2 状态码约定)。
    pub fn authenticate_admin(&self, plaintext: &str) -> Result<AuthIdentity, AuthFailure> {
        let Some(key) = self.lookup(plaintext) else {
            return Err(AuthFailure::UnknownKey);
        };
        if !key.enabled {
            return Err(AuthFailure::Disabled);
        }
        if key.is_expired(Utc::now()) {
            return Err(AuthFailure::Expired);
        }
        if key.role != KeyRole::Admin {
            return Err(AuthFailure::Forbidden);
        }
        Ok(AuthIdentity {
            key_name: key.name.clone(),
            role: key.role,
            is_master: key.is_master,
        })
    }

    /// Mark the entry matching the master plaintext as the protected master
    /// key (A2): admin role, is_master, never deletable/modifiable. Used at
    /// startup when the master key already exists in a legacy keys.json.
    pub fn mark_master(&self, plaintext: &str) {
        let fp = fingerprint_key(&self.secret, plaintext);
        let legacy_fp = format!("sha256:{}", hash_key(plaintext));
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), mark_master dropped");
                return;
            }
        };
        for fp in [fp, legacy_fp] {
            if let Some(entry) = guard.get_mut(&fp) {
                entry.is_master = true;
                entry.role = KeyRole::Admin;
            }
        }
    }

    /// Legacy boolean admin check kept for the startup master-key probe.
    pub fn authenticate(&self, plaintext: &str) -> bool {
        self.lookup(plaintext)
            .map(|k| k.enabled && !k.is_expired(Utc::now()))
            .unwrap_or(false)
    }

    /// Record token consumption against a key (by fingerprint).
    /// B3: both the cumulative total and the in-period counter advance.
    pub fn record_usage(&self, fingerprint: &str, tokens: u64) {
        if tokens == 0 {
            return;
        }
        match self.keys.write() {
            Ok(mut guard) => {
                if let Some(key) = guard.get_mut(fingerprint) {
                    key.tokens_used = key.tokens_used.saturating_add(tokens);
                    key.tokens_used_in_period = key.tokens_used_in_period.saturating_add(tokens);
                }
            }
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), usage dropped");
            }
        }
    }

    pub fn fingerprint_of(&self, plaintext: &str) -> String {
        fingerprint_key(&self.secret, plaintext)
    }

    pub fn list(&self) -> Vec<ApiKey> {
        match self.keys.read() {
            Ok(g) => g.values().cloned().collect(),
            Err(_) => {
                tracing::error!("auth keys lock poisoned (read)");
                Vec::new()
            }
        }
    }

    pub fn get_by_name(&self, name: &str) -> Option<ApiKey> {
        match self.keys.read() {
            Ok(g) => g.values().find(|k| k.name == name).cloned(),
            Err(_) => {
                tracing::error!("auth keys lock poisoned (read)");
                None
            }
        }
    }

    /// Insert or replace a key entry (name is unique).
    pub fn upsert(&self, key: ApiKey) {
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), upsert dropped");
                return;
            }
        };
        let same_name: Vec<String> = guard
            .values()
            .filter(|k| k.name == key.name)
            .map(|k| k.fingerprint.clone())
            .collect();
        for h in same_name {
            guard.remove(&h);
        }
        guard.insert(key.fingerprint.clone(), key);
    }

    pub fn remove_by_name(&self, name: &str) -> bool {
        let mut guard = match self.keys.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("auth keys lock poisoned (write), remove dropped");
                return false;
            }
        };
        let target: Option<String> = guard
            .values()
            .find(|k| k.name == name)
            .map(|k| k.fingerprint.clone());
        match target {
            Some(h) => {
                guard.remove(&h);
                true
            }
            None => false,
        }
    }

    pub fn count(&self) -> usize {
        match self.keys.read() {
            Ok(g) => g.len(),
            Err(_) => {
                tracing::error!("auth keys lock poisoned (read)");
                0
            }
        }
    }

    pub fn count_by_role(&self, role: KeyRole) -> usize {
        match self.keys.read() {
            Ok(g) => g.values().filter(|k| k.role == role).count(),
            Err(_) => {
                tracing::error!("auth keys lock poisoned (read)");
                0
            }
        }
    }
}

/// Extract the API key from a request: `Authorization: Bearer <key>` first,
/// then raw Authorization header, then `X-Api-Key`.
pub fn extract_api_key(headers: &axum::http::HeaderMap) -> String {
    if let Some(auth) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(value) = auth.to_str() {
            if let Some(stripped) = value.strip_prefix("Bearer ") {
                return stripped.to_string();
            }
            return value.to_string();
        }
    }
    if let Some(key) = headers.get("X-Api-Key") {
        if let Ok(value) = key.to_str() {
            return value.to_string();
        }
    }
    String::new()
}
