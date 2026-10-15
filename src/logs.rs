use std::collections::VecDeque;
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use serde::Serialize;

/// One recorded request handled by the data plane.
#[derive(Debug, Clone, Serialize)]
pub struct RequestLogEntry {
    pub ts: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub model: String,
    pub status: u16,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub account_id: String,
}

/// Fixed-capacity ring buffer of recent requests (v2 addition: powers the
/// /admin/api/logs endpoint shown on the dashboard logs page).
pub struct RequestLog {
    capacity: usize,
    entries: RwLock<VecDeque<RequestLogEntry>>,
}

impl RequestLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: RwLock::new(VecDeque::with_capacity(capacity)),
        }
    }

    pub fn record(&self, entry: RequestLogEntry) {
        let mut guard = self.entries.write().expect("log lock poisoned");
        if guard.len() >= self.capacity {
            guard.pop_front();
        }
        guard.push_back(entry);
    }

    /// Newest first.
    pub fn list(&self) -> Vec<RequestLogEntry> {
        self.entries
            .read()
            .expect("log lock poisoned")
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    pub fn size(&self) -> usize {
        self.entries.read().expect("log lock poisoned").len()
    }
}
