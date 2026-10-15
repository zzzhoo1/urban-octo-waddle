use std::collections::HashSet;
use std::sync::RwLock;

/// API-key authenticator shared by the data plane (/v1) and admin plane.
pub struct KeyAuth {
    keys: RwLock<HashSet<String>>,
}

impl KeyAuth {
    pub fn new(keys: Vec<String>) -> Self {
        let set = keys
            .into_iter()
            .filter(|k| !k.is_empty())
            .collect::<HashSet<_>>();
        Self {
            keys: RwLock::new(set),
        }
    }

    pub fn authenticate(&self, key: &str) -> bool {
        if key.is_empty() {
            return false;
        }
        self.keys
            .read()
            .expect("auth lock poisoned")
            .contains(key)
    }

    pub fn add(&self, key: &str) {
        if !key.is_empty() {
            self.keys
                .write()
                .expect("auth lock poisoned")
                .insert(key.to_string());
        }
    }

    pub fn remove(&self, key: &str) {
        self.keys
            .write()
            .expect("auth lock poisoned")
            .remove(key);
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
