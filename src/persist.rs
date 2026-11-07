use std::sync::Arc;
use std::time::Duration;

use crate::AppState;

/// Persist pending key changes once.
///
/// Takes the dirty fingerprints and writes the latest full snapshot. On
/// failure the fingerprints are restored so a later cycle retries instead of
/// silently dropping the changes.
pub fn flush_once(state: &Arc<AppState>) -> anyhow::Result<()> {
    let dirty = state.keys.take_dirty();
    if dirty.is_empty() {
        return Ok(());
    }
    match state.key_store.save_keys_with(|| state.keys.list()) {
        Ok(()) => Ok(()),
        Err(e) => {
            state.keys.restore_dirty(dirty);
            Err(e)
        }
    }
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
