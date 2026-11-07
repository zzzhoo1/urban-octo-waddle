use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, RwLock};
use std::time::Duration;

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
    /// B4: actual account that served the request (final attempt).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub account_id: String,
    /// Time to first token (streaming: first SSE frame; non-streaming:
    /// first byte of the upstream response body). 0 when unknown.
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
    /// B4: resolution layer (user_rule / downgrade / builtin_tier / passthrough).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub route_via: String,
    /// B4: the upstream model actually called (client name stays in `model`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub resolved_model: String,
    /// B4: number of attempts including the first (>= 1).
    #[serde(default, skip_serializing_if = "is_one")]
    pub attempts: u32,
    /// B4: account ids tried in order (empty when a single attempt).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry_account_ids: Vec<String>,
    /// B4: log_only downgrade observation flag (B2).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub downgrade_observed: bool,
}

impl Default for RequestLogEntry {
    fn default() -> Self {
        Self {
            ts: Utc::now(),
            method: String::new(),
            path: String::new(),
            model: String::new(),
            status: 0,
            latency_ms: 0,
            session_id: String::new(),
            account_id: String::new(),
            ttft_ms: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            credit: None,
            cached_tokens: None,
            source_ip: String::new(),
            key_name: String::new(),
            route_via: String::new(),
            resolved_model: String::new(),
            attempts: 1,
            retry_account_ids: Vec::new(),
            downgrade_observed: false,
        }
    }
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

fn is_one(v: &u32) -> bool {
    *v <= 1
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
    /// B4: persistence degradation flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
    /// B4: retention window actually covered by the data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_from: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_to: Option<DateTime<Utc>>,
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

/// B4: query filter for logs and stats.
#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub model: Option<String>,
    pub key_name: Option<String>,
    /// "2xx" / "4xx" / "5xx"
    pub status_class: Option<String>,
    /// list pagination
    pub limit: Option<u64>,
    pub before: Option<DateTime<Utc>>,
}

/// Request log with SQLite persistence (B4) and an in-memory ring buffer
/// as hot data / degraded fallback.
pub struct RequestLog {
    capacity: usize,
    entries: RwLock<VecDeque<RequestLogEntry>>,
    /// B4: async SQLite writer.
    writer: Option<LogWriter>,
    degraded: AtomicBool,
}

impl RequestLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: RwLock::new(VecDeque::with_capacity(capacity)),
            writer: None,
            degraded: AtomicBool::new(false),
        }
    }

    /// B4: enable SQLite persistence at `path` with retention settings.
    /// Spawns a background flush task; failures degrade to memory-only mode.
    pub fn with_db(mut self, path: &str, retention_days: u64, max_rows: u64) -> Self {
        match LogWriter::new(path, retention_days, max_rows) {
            Some(w) => {
                self.writer = Some(w);
                tracing::info!(path = %path, "audit log persistence enabled");
            }
            None => {
                tracing::warn!("audit log persistence unavailable, memory-only mode");
            }
        }
        self
    }

    pub fn is_degraded(&self) -> bool {
        self.degraded.load(Ordering::Relaxed)
    }

    pub fn record(&self, entry: RequestLogEntry) {
        // Memory buffer always updated (hot data + degraded fallback).
        match self.entries.write() {
            Ok(mut guard) => {
                if guard.len() >= self.capacity {
                    guard.pop_front();
                }
                guard.push_back(entry.clone());
            }
            Err(_) => {
                tracing::error!("request log lock poisoned (write), entry dropped");
            }
        }
        // B4: async batched persistence; never blocks the request path.
        if let Some(w) = &self.writer {
            if let Err(e) = w.send(entry) {
                if !self.is_degraded() {
                    tracing::warn!("audit log persistence degraded: {}", e);
                }
                self.degraded.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Newest first. B4: reads from SQLite when available (with query
    /// filters and pagination), falling back to the memory buffer.
    pub fn list(&self, q: &LogQuery) -> Vec<RequestLogEntry> {
        if let Some(w) = &self.writer {
            match w.query(q) {
                Ok(v) => {
                    self.degraded.store(false, Ordering::Relaxed);
                    return v;
                }
                Err(e) => {
                    tracing::error!("audit log query failed, falling back to memory: {}", e);
                    self.degraded.store(true, Ordering::Relaxed);
                }
            }
        }
        let guard = match self.entries.read() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("request log lock poisoned (read)");
                return Vec::new();
            }
        };
        let mut out: Vec<RequestLogEntry> = guard.iter().rev().cloned().collect();
        if let Some(l) = q.limit {
            out.truncate(l as usize);
        }
        out
    }

    pub fn size(&self) -> usize {
        match self.entries.read() {
            Ok(g) => g.len(),
            Err(_) => 0,
        }
    }

    /// Aggregate stats. B4: computed from SQLite when available (with
    /// optional time range), falling back to the memory buffer.
    pub fn stats(&self, q: &LogQuery) -> StatsResponse {
        let entries: Vec<RequestLogEntry> = if let Some(w) = &self.writer {
            match w.query(&LogQuery {
                from: q.from,
                to: q.to,
                ..Default::default()
            }) {
                Ok(v) => {
                    self.degraded.store(false, Ordering::Relaxed);
                    v
                }
                Err(e) => {
                    tracing::error!("audit stats query failed, memory fallback: {}", e);
                    self.degraded.store(true, Ordering::Relaxed);
                    self.list(&LogQuery::default())
                }
            }
        } else {
            self.list(&LogQuery::default())
        };
        let degraded = self.is_degraded();
        let window_from = entries.last().map(|e| e.ts);
        let window_to = entries.first().map(|e| e.ts);
        let mut resp = aggregate(&entries);
        resp.degraded = Some(degraded);
        resp.degraded_reason = if degraded {
            Some("sqlite unavailable, memory buffer only".to_string())
        } else {
            None
        };
        resp.window_from = window_from;
        resp.window_to = window_to;
        resp
    }
}

/// Aggregate a list of entries into the stats response.
fn aggregate(entries: &[RequestLogEntry]) -> StatsResponse {
    let mut totals = StatsBucket::default();
    let mut by_day: BTreeMap<String, StatsBucket> = BTreeMap::new();
    let mut by_model: BTreeMap<String, StatsBucket> = BTreeMap::new();
    let mut by_key: BTreeMap<String, StatsBucket> = BTreeMap::new();

    for e in entries {
        totals.add(e);
        let day = format!("{:04}-{:02}-{:02}", e.ts.year(), e.ts.month(), e.ts.day());
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
        degraded: None,
        degraded_reason: None,
        window_from: None,
        window_to: None,
    }
}

/// B4: background SQLite writer. Entries are batched (200 rows / 2s,
/// whichever first) and flushed off the request path. A daily cleanup task
/// enforces the retention policy.
struct LogWriter {
    tx: mpsc::Sender<RequestLogEntry>,
    /// Database path for query-side connections (shared across threads).
    db_path: String,
}

impl LogWriter {
    const BATCH_ROWS: usize = 200;
    const FLUSH_INTERVAL: Duration = Duration::from_secs(2);

    fn new(path: &str, retention_days: u64, max_rows: u64) -> Option<Self> {
        // Open (and migrate) the database on this thread first so startup
        // failures surface immediately.
        let conn = open_db(path)?;
        let (tx, rx) = mpsc::channel::<RequestLogEntry>();
        std::thread::Builder::new()
            .name("audit-log-writer".to_string())
            .spawn(move || writer_loop(rx, conn, retention_days, max_rows))
            .ok()?;
        Some(Self {
            tx,
            db_path: path.to_string(),
        })
    }

    fn send(&self, entry: RequestLogEntry) -> Result<(), String> {
        self.tx.send(entry).map_err(|e| e.to_string())
    }

    /// Query entries from SQLite. Opens a fresh short-lived connection on
    /// the caller thread (WAL mode keeps concurrent reads cheap).
    fn query(&self, q: &LogQuery) -> Result<Vec<RequestLogEntry>, String> {
        let conn = rusqlite::Connection::open(&self.db_path).map_err(|e| e.to_string())?;
        query_entries(&conn, q)
    }
}

fn writer_loop(
    rx: mpsc::Receiver<RequestLogEntry>,
    conn: rusqlite::Connection,
    retention_days: u64,
    max_rows: u64,
) {
    let mut pending: Vec<RequestLogEntry> = Vec::with_capacity(LogWriter::BATCH_ROWS);
    let mut last_cleanup = Utc::now();
    loop {
        // Drain whatever is available without blocking indefinitely.
        let timeout = if pending.is_empty() {
            // Idle: block until something arrives.
            match rx.recv() {
                Ok(e) => pending.push(e),
                Err(_) => break,
            }
            continue;
        };
        let deadline = std::time::Instant::now() + LogWriter::FLUSH_INTERVAL;
        while pending.len() < LogWriter::BATCH_ROWS {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            match rx.recv_timeout(deadline - now) {
                Ok(e) => pending.push(e),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    flush_batch(&conn, &mut pending);
                    return;
                }
            }
        }
        let _ = timeout;
        flush_batch(&conn, &mut pending);
        // Daily retention cleanup.
        if (Utc::now() - last_cleanup).num_hours() >= 24 {
            last_cleanup = Utc::now();
            if let Err(e) = cleanup(&conn, retention_days, max_rows) {
                tracing::error!("audit log cleanup failed: {}", e);
            }
        }
    }
    flush_batch(&conn, &mut pending);
}

fn flush_batch(conn: &rusqlite::Connection, pending: &mut Vec<RequestLogEntry>) {
    if pending.is_empty() {
        return;
    }
    if let Err(e) = conn.execute_batch("BEGIN") {
        tracing::error!("audit log batch begin failed: {}", e);
        pending.clear();
        return;
    }
    let mut ok = true;
    for e in pending.iter() {
        if let Err(err) = insert_entry(conn, e) {
            tracing::error!("audit log insert failed: {}", err);
            ok = false;
            break;
        }
    }
    let _ = conn.execute_batch(if ok { "COMMIT" } else { "ROLLBACK" });
    pending.clear();
}

fn insert_entry(conn: &rusqlite::Connection, e: &RequestLogEntry) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO request_logs (ts, method, path, model, resolved_model, route_via, status, \
         latency_ms, ttft_ms, prompt_tokens, completion_tokens, total_tokens, credit, \
         cached_tokens, source_ip, key_name, account_id, attempts, retry_account_ids, \
         session_id, downgrade_observed) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
         ?17, ?18, ?19, ?20, ?21)",
        rusqlite::params![
            e.ts.to_rfc3339(),
            e.method,
            e.path,
            e.model,
            e.resolved_model,
            e.route_via,
            e.status,
            e.latency_ms,
            e.ttft_ms,
            e.prompt_tokens,
            e.completion_tokens,
            e.total_tokens,
            e.credit,
            e.cached_tokens,
            e.source_ip,
            e.key_name,
            e.account_id,
            e.attempts,
            serde_json::to_string(&e.retry_account_ids).unwrap_or_default(),
            e.session_id,
            e.downgrade_observed,
        ],
    )?;
    Ok(())
}

fn open_db(path: &str) -> Option<rusqlite::Connection> {
    let conn = match rusqlite::Connection::open(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("open logs db failed: {}", e);
            return None;
        }
    };
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");
    if let Err(e) = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts TEXT NOT NULL,
            method TEXT NOT NULL,
            path TEXT NOT NULL,
            model TEXT NOT NULL,
            resolved_model TEXT NOT NULL DEFAULT '',
            route_via TEXT NOT NULL DEFAULT '',
            status INTEGER NOT NULL,
            latency_ms INTEGER NOT NULL DEFAULT 0,
            ttft_ms INTEGER NOT NULL DEFAULT 0,
            prompt_tokens INTEGER NOT NULL DEFAULT 0,
            completion_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            credit REAL,
            cached_tokens INTEGER,
            source_ip TEXT NOT NULL DEFAULT '',
            key_name TEXT NOT NULL DEFAULT '',
            account_id TEXT NOT NULL DEFAULT '',
            attempts INTEGER NOT NULL DEFAULT 1,
            retry_account_ids TEXT NOT NULL DEFAULT '[]',
            session_id TEXT NOT NULL DEFAULT '',
            downgrade_observed INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_logs_ts ON request_logs (ts DESC);
        CREATE INDEX IF NOT EXISTS idx_logs_model ON request_logs (model);
        CREATE INDEX IF NOT EXISTS idx_logs_key ON request_logs (key_name);
        CREATE INDEX IF NOT EXISTS idx_logs_status ON request_logs (status);
        CREATE INDEX IF NOT EXISTS idx_logs_account ON request_logs (account_id);",
    ) {
        tracing::error!("init logs db schema failed: {}", e);
        return None;
    }
    Some(conn)
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestLogEntry> {
    let ts: String = row.get("ts")?;
    let retry_ids: String = row.get("retry_account_ids")?;
    Ok(RequestLogEntry {
        ts: DateTime::parse_from_rfc3339(&ts)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_default(),
        method: row.get("method")?,
        path: row.get("path")?,
        model: row.get("model")?,
        status: row.get::<_, i64>("status")? as u16,
        latency_ms: row.get("latency_ms")?,
        session_id: row.get("session_id")?,
        account_id: row.get("account_id")?,
        ttft_ms: row.get("ttft_ms")?,
        prompt_tokens: row.get("prompt_tokens")?,
        completion_tokens: row.get("completion_tokens")?,
        total_tokens: row.get("total_tokens")?,
        credit: row.get("credit")?,
        cached_tokens: row.get("cached_tokens")?,
        source_ip: row.get("source_ip")?,
        key_name: row.get("key_name")?,
        route_via: row.get("route_via")?,
        resolved_model: row.get("resolved_model")?,
        attempts: row.get::<_, i64>("attempts")? as u32,
        retry_account_ids: serde_json::from_str(&retry_ids).unwrap_or_default(),
        downgrade_observed: row.get::<_, i64>("downgrade_observed")? != 0,
    })
}

fn query_entries(
    conn: &rusqlite::Connection,
    q: &LogQuery,
) -> Result<Vec<RequestLogEntry>, String> {
    let mut sql = String::from(
        "SELECT ts, method, path, model, resolved_model, route_via, status, latency_ms, \
         ttft_ms, prompt_tokens, completion_tokens, total_tokens, credit, cached_tokens, \
         source_ip, key_name, account_id, attempts, retry_account_ids, session_id, \
         downgrade_observed FROM request_logs WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(from) = q.from {
        sql.push_str(&format!(" AND ts >= ?{}", params.len() + 1));
        params.push(Box::new(from.to_rfc3339()));
    }
    if let Some(to) = q.to {
        sql.push_str(&format!(" AND ts <= ?{}", params.len() + 1));
        params.push(Box::new(to.to_rfc3339()));
    }
    if let Some(before) = q.before {
        sql.push_str(&format!(" AND ts < ?{}", params.len() + 1));
        params.push(Box::new(before.to_rfc3339()));
    }
    if let Some(model) = &q.model {
        sql.push_str(&format!(" AND model = ?{}", params.len() + 1));
        params.push(Box::new(model.clone()));
    }
    if let Some(key) = &q.key_name {
        sql.push_str(&format!(" AND key_name = ?{}", params.len() + 1));
        params.push(Box::new(key.clone()));
    }
    if let Some(class) = &q.status_class {
        let (lo, hi) = match class.as_str() {
            "2xx" => (200, 299),
            "4xx" => (400, 499),
            "5xx" => (500, 599),
            _ => (0, 999),
        };
        sql.push_str(&format!(" AND status BETWEEN ?{} AND ?{}", params.len() + 1, params.len() + 2));
        params.push(Box::new(lo));
        params.push(Box::new(hi));
    }
    sql.push_str(" ORDER BY ts DESC");
    let limit = q.limit.unwrap_or(200).min(1000);
    sql.push_str(&format!(" LIMIT {}", limit));
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())), row_to_entry)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn cleanup(conn: &rusqlite::Connection, retention_days: u64, max_rows: u64) -> Result<(), String> {
    let cutoff = (Utc::now() - chrono::Duration::days(retention_days as i64)).to_rfc3339();
    conn.execute("DELETE FROM request_logs WHERE ts < ?1", rusqlite::params![cutoff])
        .map_err(|e| e.to_string())?;
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM request_logs", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if count > max_rows as i64 {
        conn.execute(
            "DELETE FROM request_logs WHERE id IN (
                SELECT id FROM request_logs ORDER BY ts ASC LIMIT ?1
            )",
            rusqlite::params![count - max_rows as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
