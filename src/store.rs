use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use crate::account::AccountData;
use crate::auth::ApiKey;
use crate::router::RouterConfig;

/// JSON-file account store. Saves atomically via tmp-file + rename,
/// mirroring the legacy Go store so accounts.json stays compatible.
///
/// 所有写路径（accounts / keys / router）共用同一把写锁 `mu`，
/// 避免并发写入同一个 *.json.tmp 导致 rename 失败或互相覆盖。
pub struct JsonFileStore {
    path: PathBuf,
    mu: Mutex<()>,
}

impl JsonFileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mu: Mutex::new(()),
        }
    }

    /// 获取写锁。`Mutex<()>` 不保护任何数据，锁中毒时直接恢复即可，
    /// 不会读到被破坏的状态，也避免因为一次 panic 让后续落盘全部失效。
    fn lock_writes(&self) -> MutexGuard<'_, ()> {
        self.mu.lock().unwrap_or_else(|poisoned| {
            tracing::error!("store lock poisoned, recovering");
            poisoned.into_inner()
        })
    }

    /// 调用方必须已持有 `mu`，本函数自身不加锁（std Mutex 不可重入）。
    fn atomic_write_locked(&self, path: &PathBuf, data: &[u8]) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn save(&self, accounts: &[AccountData]) -> anyhow::Result<()> {
        let _guard = match self.mu.lock() {
            Ok(g) => g,
            Err(_) => {
                // A1: 持久化写路径锁中毒：放弃本次写入，不影响请求
                tracing::error!("account store lock poisoned (write), save skipped");
                return Ok(());
            }
        };
        let data = serde_json::to_vec_pretty(accounts)?;
        self.atomic_write_locked(&self.path.clone(), &data)
    }

    pub fn load(&self) -> anyhow::Result<Vec<AccountData>> {
        let _guard = match self.mu.lock() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("account store lock poisoned (read)");
                return Ok(Vec::new());
            }
        };
        match std::fs::read(&self.path) {
            Ok(data) => Ok(serde_json::from_slice(&data)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// keys.json lives next to accounts.json.
    fn keys_path(&self) -> PathBuf {
        self.path
            .parent()
            .map(|d| d.join("keys.json"))
            .unwrap_or_else(|| PathBuf::from("keys.json"))
    }

    /// router.json lives next to accounts.json.
    fn router_path(&self) -> PathBuf {
        self.path
            .parent()
            .map(|d| d.join("router.json"))
            .unwrap_or_else(|| PathBuf::from("router.json"))
    }

    pub fn save_keys(&self, keys: &[ApiKey]) -> anyhow::Result<()> {
        let data = serde_json::to_vec_pretty(keys)?;
        let _guard = self.lock_writes();
        self.atomic_write_locked(&self.keys_path(), &data)
    }

    /// 在持有写锁之后才调用 `snapshot` 取最新快照再落盘，
    /// 保证最后一个获得锁的写入者持久化的一定是最新的内存状态，
    /// 不会被先取快照、后拿到锁的旧快照覆盖。
    pub fn save_keys_with<F>(&self, snapshot: F) -> anyhow::Result<()>
    where
        F: FnOnce() -> Vec<ApiKey>,
    {
        let _guard = self.lock_writes();
        let keys = snapshot();
        let data = serde_json::to_vec_pretty(&keys)?;
        self.atomic_write_locked(&self.keys_path(), &data)
    }

    pub fn load_keys(&self) -> anyhow::Result<Vec<ApiKey>> {
        match std::fs::read(self.keys_path()) {
            Ok(data) => Ok(serde_json::from_slice(&data)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_router(&self, cfg: &RouterConfig) -> anyhow::Result<()> {
        let data = serde_json::to_vec_pretty(cfg)?;
        let _guard = self.lock_writes();
        self.atomic_write_locked(&self.router_path(), &data)
    }

    pub fn load_router(&self) -> anyhow::Result<RouterConfig> {
        match std::fs::read(self.router_path()) {
            Ok(data) => Ok(serde_json::from_slice(&data)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RouterConfig::default()),
            Err(e) => Err(e.into()),
        }
    }
}
