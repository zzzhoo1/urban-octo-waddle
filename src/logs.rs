use std::collections::{BTreeMap, VecDeque};
use std::sync::RwLock;

use chrono::{DateTime, Datelike, Utc};
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
    /// Time to first token for streaming requests (0 when unknown).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ttft_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub prompt_tokens: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub completion_tokens: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub total_tokens: u64,
    /// Actual credit deducted (from upstream usage.credit). None = the
    /// upstream did not report it; the panel renders "—", never 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<f64>,
    /// Prompt tokens served from cache. None = upstream did not report it;
    /// the panel renders "—", distinct from an actual 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    /// Client source IP (from connection or X-Forwarded-For).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_ip: String,
    /// Name of the API key used (multi-key support).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key_name: String,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// Fixed-capacity ring buffer of recent requests.
pub struct RequestLog {
    capacity: usize,
    entries: RwLock<VecDeque<RequestLogEntry>>,
}

/// Aggregated statistics for a time bucket.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatsBucket {
    pub requests: u64,
    pub errors: u64,
    pub total_tokens: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub latency_sum_ms: u64,
    pub ttft_sum_ms: u64,
    pub ttft_count: u64,
}

impl StatsBucket {
    pub fn add(&mut self, e: &RequestLogEntry) {
        self.requests += 1;
        if e.status >= 400 {
            self.errors += 1;
        }
        self.total_tokens += e.total_tokens;
        self.prompt_tokens += e.prompt_tokens;
        self.completion_tokens += e.completion_tokens;
        self.latency_sum_ms += e.latency_ms;
        if e.ttft_ms > 0 {
            self.ttft_sum_ms += e.ttft_ms;
            self.ttft_count += 1;
        }
    }

    pub fn avg_latency(&self) -> u64 {
        if self.requests == 0 {
            0
        } else {
            self.latency_sum_ms / self.requests
        }
    }

    pub fn avg_ttft(&self) -> u64 {
        if self.ttft_count == 0 {
            0
        } else {
            self.ttft_sum_ms / self.ttft_count
        }
    }
}

/// Overall stats response for the dashboard.
#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub totals: StatsTotals,
    /// Daily buckets keyed by ISO date (YYYY-MM-DD), ascending.
    pub by_day: Vec<DayStats>,
    /// Per-model totals, descending by requests.
    pub by_model: Vec<ModelStats>,
    /// Per-key totals, descending by requests.
    pub by_key: Vec<KeyStats>,
}

#[derive(Debug, Serialize)]
pub struct StatsTotals {
    pub requests: u64,
    pub errors: u64,
    pub success_rate: f64,
    pub total_tokens: u64,
    pub avg_latency_ms: u64,
    pub avg_ttft_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct DayStats {
    pub date: String,
    pub requests: u64,
    pub errors: u64,
    pub total_tokens: u64,
    pub avg_latency_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct ModelStats {
    pub model: String,
    pub requests: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Serialize)]
pub struct KeyStats {
    pub key_name: String,
    pub requests: u64,
    pub total_tokens: u64,
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

    /// Aggregate stats over all retained entries.
    pub fn stats(&self) -> StatsResponse {
        let guard = self.entries.read().expect("log lock poisoned");
        let mut totals = StatsBucket::default();
        let mut by_day: BTreeMap<String, StatsBucket> = BTreeMap::new();
        let mut by_model: BTreeMap<String, StatsBucket> = BTreeMap::new();
        let mut by_key: BTreeMap<String, StatsBucket> = BTreeMap::new();

        for e in guard.iter() {
            totals.add(e);
            let day = format!(
                "{:04}-{:02}-{:02}",
                e.ts.year(),
                e.ts.month(),
                e.ts.day()
            );
            by_day.entry(day).or_default().add(e);
            if !e.model.is_empty() {
                by_model.entry(e.model.clone()).or_default().add(e);
            }
            let key = if e.key_name.is_empty() {
                "(unknown)".to_string()
            } else {
                e.key_name.clone()
            };
            by_key.entry(key).or_default().add(e);
        }

        let success_rate = if totals.requests == 0 {
            1.0
        } else {
            (totals.requests - totals.errors) as f64 / totals.requests as f64
        };

        let mut model_stats: Vec<ModelStats> = by_model
            .into_iter()
            .map(|(model, b)| ModelStats {
                model,
                requests: b.requests,
                total_tokens: b.total_tokens,
            })
            .collect();
        model_stats.sort_by(|a, b| b.requests.cmp(&a.requests));

        let mut key_stats: Vec<KeyStats> = by_key
            .into_iter()
            .map(|(key_name, b)| KeyStats {
                key_name,
                requests: b.requests,
                total_tokens: b.total_tokens,
            })
            .collect();
        key_stats.sort_by(|a, b| b.requests.cmp(&a.requests));

        StatsResponse {
            totals: StatsTotals {
                requests: totals.requests,
                errors: totals.errors,
                success_rate,
                total_tokens: totals.total_tokens,
                avg_latency_ms: totals.avg_latency(),
                avg_ttft_ms: totals.avg_ttft(),
            },
            by_day: by_day
                .into_iter()
                .map(|(date, b)| DayStats {
                    date,
                    requests: b.requests,
                    errors: b.errors,
                    total_tokens: b.total_tokens,
                    avg_latency_ms: b.avg_latency(),
                })
                .collect(),
            by_model: model_stats,
            by_key: key_stats,
        }
    }
}
