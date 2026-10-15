use std::collections::HashMap;
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A named API key with optional expiry, per-model allowlist and token quota.
/// The plaintext key is stored hashed (SHA-256) so the file can be backed up
/// without exposing usable secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    /// Display name shown in logs and the panel.
    pub name: String,
    /// SHA-256 hex of the plaintext key.
    pub key_hash: String,
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
}

fn default_true() -> bool {
    true
}

impl ApiKey {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.map(|t| now > t).unwrap_or(false)
    }

    pub fn quota_exceeded(&self) -> bool {
        self.token_quota > 0 && self.tokens_used >= self.token_quota
    }

    pub fn model_allowed(&self, model: &str) -> bool {
        self.model_allowlist.is_empty() || self.model_allowlist.iter().any(|m| m == model)
    }
}

pub fn hash_key(plaintext: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

pub fn mask_key(plaintext: &str) -> String {
    if plaintext.len() <= 8 {
        return "****".to_string();
    }
    format!("{}...{}", &plaintext[..4], &plaintext[plaintext.len() - 4..])
}

/// Result of a successful authentication.
#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub key_name: String,
}

/// Authentication errors surfaced to callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    UnknownKey,
    Expired,
    Disabled,
    QuotaExceeded,
    ModelNotAllowed,
}

/// Multi-key authenticator shared by the data plane (/v1) and admin plane.
/// Keys persist to a JSON file; admin-plane access accepts any enabled,
/// unexpired key (master key concept: the first key acts as admin).
pub struct KeyAuth {
    keys: RwLock<HashMap<String, ApiKey>>, // key_hash -> ApiKey
}

impl KeyAuth {
    pub fn new(master_plaintext: Vec<String>) -> Self {
        let mut map = HashMap::new();
        for (i, k) in master_plaintext.into_iter().filter(|k| !k.is_empty()).enumerate() {
            let name = if i == 0 { "master".to_string() } else { format!("key-{}", i) };
            let hash = hash_key(&k);
            map.entry(hash.clone()).or_insert(ApiKey {
                name,
                key_hash: hash,
                hint: mask_key(&k),
                created_at: Some(Utc::now()),
                expires_at: None,
                token_quota: 0,
                tokens_used: 0,
                model_allowlist: Vec::new(),
                enabled: true,
            });
        }
        Self {
            keys: RwLock::new(map),
        }
    }

    pub fn from_entries(entries: Vec<ApiKey>) -> Self {
        let map = entries
            .into_iter()
            .map(|k| (k.key_hash.clone(), k))
            .collect();
        Self {
            keys: RwLock::new(map),
        }
    }

    /// Authenticate a data-plane caller for a specific model.
    pub fn authenticate_for_model(
        &self,
        plaintext: &str,
        model: &str,
    ) -> Result<AuthIdentity, AuthFailure> {
        if plaintext.is_empty() {
            return Err(AuthFailure::UnknownKey);
        }
        let hash = hash_key(plaintext);
        let guard = self.keys.read().expect("auth lock poisoned");
        let Some(key) = guard.get(&hash) else {
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
        if !model.is_empty() && !key.model_allowed(model) {
            return Err(AuthFailure::ModelNotAllowed);
        }
        Ok(AuthIdentity {
            key_name: key.name.clone(),
        })
    }

    /// Admin-plane auth: any valid (enabled, unexpired) key passes.
    pub fn authenticate(&self, plaintext: &str) -> bool {
        if plaintext.is_empty() {
            return false;
        }
        let hash = hash_key(plaintext);
        let guard = self.keys.read().expect("auth lock poisoned");
        guard
            .get(&hash)
            .map(|k| k.enabled && !k.is_expired(Utc::now()))
            .unwrap_or(false)
    }

    /// Record token consumption against a key (by hash).
    pub fn record_usage(&self, plaintext_hash: &str, tokens: u64) {
        if tokens == 0 {
            return;
        }
        let mut guard = self.keys.write().expect("auth lock poisoned");
        if let Some(key) = guard.get_mut(plaintext_hash) {
            key.tokens_used = key.tokens_used.saturating_add(tokens);
        }
    }

    pub fn hash_of(plaintext: &str) -> String {
        hash_key(plaintext)
    }

    pub fn list(&self) -> Vec<ApiKey> {
        self.keys
            .read()
            .expect("auth lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    pub fn get_by_name(&self, name: &str) -> Option<ApiKey> {
        self.keys
            .read()
            .expect("auth lock poisoned")
            .values()
            .find(|k| k.name == name)
            .cloned()
    }

    /// Insert or replace a key entry (name is unique).
    pub fn upsert(&self, key: ApiKey) {
        let mut guard = self.keys.write().expect("auth lock poisoned");
        // Enforce unique names: drop any existing entry with the same name.
        let same_name: Vec<String> = guard
            .values()
            .filter(|k| k.name == key.name)
            .map(|k| k.key_hash.clone())
            .collect();
        for h in same_name {
            guard.remove(&h);
        }
        guard.insert(key.key_hash.clone(), key);
    }

    pub fn remove_by_name(&self, name: &str) -> bool {
        let mut guard = self.keys.write().expect("auth lock poisoned");
        let target: Option<String> = guard
            .values()
            .find(|k| k.name == name)
            .map(|k| k.key_hash.clone());
        match target {
            Some(h) => {
                guard.remove(&h);
                true
            }
            None => false,
        }
    }

    pub fn count(&self) -> usize {
        self.keys.read().expect("auth lock poisoned").len()
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
