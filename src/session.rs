use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

struct Entry {
    account_id: String,
    expires_at: Instant,
}

/// Sticky-session manager: binds a client session id to an account so
/// multi-turn conversations keep hitting the same upstream account.
pub struct SessionManager {
    entries: RwLock<HashMap<String, Entry>>,
    ttl: Duration,
    max_size: usize,
}

impl SessionManager {
    pub fn new(ttl: Duration, max_size: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            ttl,
            max_size,
        }
    }

    /// Look up the account bound to a session; expired entries are removed.
    pub fn get(&self, session_id: &str) -> Option<String> {
        let mut guard = match self.entries.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("session lock poisoned (write)");
                return None;
            }
        };
        match guard.get(session_id) {
            Some(entry) if Instant::now() < entry.expires_at => Some(entry.account_id.clone()),
            Some(_) => {
                guard.remove(session_id);
                None
            }
            None => None,
        }
    }

    /// Bind a session to an account, evicting the oldest entry when full.
    pub fn bind(&self, session_id: &str, account_id: &str) {
        let mut guard = match self.entries.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("session lock poisoned (write), bind dropped");
                return;
            }
        };
        guard.insert(
            session_id.to_string(),
            Entry {
                account_id: account_id.to_string(),
                expires_at: Instant::now() + self.ttl,
            },
        );
        if guard.len() > self.max_size {
            if let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(id, _)| id.clone())
            {
                guard.remove(&oldest);
            }
        }
    }

    pub fn unbind(&self, session_id: &str) {
        if let Ok(mut g) = self.entries.write() {
            g.remove(session_id);
        } else {
            tracing::error!("session lock poisoned (write)");
        }
    }

    pub fn size(&self) -> usize {
        match self.entries.read() {
            Ok(g) => g.len(),
            Err(_) => 0,
        }
    }

    /// Remove all expired entries, returning how many were purged.
    pub fn purge_expired(&self) -> usize {
        let now = Instant::now();
        let mut guard = match self.entries.write() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("session lock poisoned (write)");
                return 0;
            }
        };
        let before = guard.len();
        guard.retain(|_, entry| now < entry.expires_at);
        before - guard.len()
    }
}
