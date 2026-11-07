use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::Connection;

/// Per-key usage record persisted in SQLite (issue #3 suggestion 4).
#[derive(Debug, Clone)]
pub struct UsageRow {
    pub fingerprint: String,
    pub tokens_used: u64,
    pub tokens_used_in_period: u64,
    pub period_start: Option<DateTime<Utc>>,
}

/// SQLite-backed usage store. The in-memory `KeyAuth` counters remain the
/// hot path for quota checks; this store is the durable authority and is
/// flushed in batches by the background persist loop.
pub struct UsageStore {
    conn: Mutex<Connection>,
}

impl UsageStore {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS usage (
                fingerprint TEXT PRIMARY KEY,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                tokens_used_in_period INTEGER NOT NULL DEFAULT 0,
                period_start INTEGER
            )",
            [],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn load_all(&self) -> anyhow::Result<Vec<UsageRow>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT fingerprint, tokens_used, tokens_used_in_period, period_start FROM usage",
        )?;
        let rows = stmt.query_map([], |row| {
            let fp: String = row.get(0)?;
            let used: u64 = row.get(1)?;
            let used_in_period: u64 = row.get(2)?;
            let start: Option<i64> = row.get(3)?;
            Ok(UsageRow {
                fingerprint: fp,
                tokens_used: used,
                tokens_used_in_period: used_in_period,
                period_start: start.and_then(|ts| DateTime::from_timestamp(ts, 0)),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Upsert the absolute counters for one key (called by the background
    /// flush with the latest in-memory values).
    pub fn upsert(
        &self,
        fingerprint: &str,
        tokens_used: u64,
        tokens_used_in_period: u64,
        period_start: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO usage (fingerprint, tokens_used, tokens_used_in_period, period_start)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(fingerprint) DO UPDATE SET
                tokens_used = excluded.tokens_used,
                tokens_used_in_period = excluded.tokens_used_in_period,
                period_start = excluded.period_start",
            rusqlite::params![
                fingerprint,
                tokens_used as i64,
                tokens_used_in_period as i64,
                period_start.map(|d| d.timestamp())
            ],
        )?;
        Ok(())
    }
}
