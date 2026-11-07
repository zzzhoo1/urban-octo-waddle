use std::sync::Arc;
use std::time::Duration;

use crate::AppState;

/// Persist pending key changes once.
///
/// Takes the dirty fingerprints and writes the latest full snapshot. On
/// failure the fingerprints are restored so a later cycle retries instead of
/// silently dropping the changes.
pub fn flush_once(state: &Arc<AppState>) -> anyhow::Result<()> {
    let mut result = Ok(());

    // keys.json: structural changes only (lazy hash upgrade, admin edits).
    let dirty = state.keys.take_dirty();
    if !dirty.is_empty() {
        if let Err(e) = state.key_store.save_keys_with(|| state.keys.list()) {
            state.keys.restore_dirty(dirty);
            result = Err(e);
        }
    }

    // usage.db: batched absolute counters for changed keys.
    let usage_dirty = state.keys.take_usage_dirty();
    if !usage_dirty.is_empty() {
        let mut failed: Option<anyhow::Error> = None;
        for fp in &usage_dirty {
            let Some((used, used_in_period, start)) = state.keys.usage_snapshot(fp) else {
                continue;
            };
            if let Err(e) = state.usage.upsert(fp, used, used_in_period, start) {
                failed = Some(e);
                break;
            }
        }
        if let Some(e) = failed {
            state.keys.restore_usage_dirty(usage_dirty);
            result = Err(e);
        }
    }

    result
}

/// Handle for the background persist loop; supports graceful shutdown.
pub struct PersistHandle {
    task: tokio::task::JoinHandle<()>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl PersistHandle {
    /// Start the background loop: every `interval`, persist pending key
    /// changes off the async runtime (spawn_blocking).
    pub fn start(state: Arc<AppState>, interval: Duration) -> Self {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            let mut rx = shutdown_rx;
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {}
                    _ = rx.changed() => break,
                }
                let st = state.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if let Err(e) = flush_once(&st) {
                        tracing::error!("background keys.json persist failed: {}", e);
                    }
                })
                .await;
            }
        });
        Self {
            task,
            shutdown: shutdown_tx,
        }
    }

    /// Stop the loop (waiting for any in-flight write), then perform one
    /// final flush so in-memory changes are not lost on graceful exit.
    pub async fn shutdown(self, state: Arc<AppState>) {
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
        shutdown_flush(state).await;
    }
}

/// Final flush at shutdown; waits for the write to complete.
async fn shutdown_flush(state: Arc<AppState>) {
    let _ = tokio::task::spawn_blocking(move || {
        if let Err(e) = flush_once(&state) {
            tracing::error!("shutdown keys.json flush failed: {}", e);
        }
    })
    .await;
}
