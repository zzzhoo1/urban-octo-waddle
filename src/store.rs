use std::path::PathBuf;
use std::sync::Mutex;

use crate::account::AccountData;
use crate::auth::ApiKey;
use crate::router::RouterConfig;

/// JSON-file account store. Saves atomically via tmp-file + rename,
/// mirroring the legacy Go store so accounts.json stays compatible.
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

    fn atomic_write(&self, path: &PathBuf, data: &[u8]) -> anyhow::Result<()> {
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
        let _guard = self.mu.lock().expect("store lock poisoned");
        let data = serde_json::to_vec_pretty(accounts)?;
        self.atomic_write(&self.path.clone(), &data)
    }

    pub fn load(&self) -> anyhow::Result<Vec<AccountData>> {
        let _guard = self.mu.lock().expect("store lock poisoned");
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
        self.atomic_write(&self.keys_path(), &data)
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
        self.atomic_write(&self.router_path(), &data)
    }

    pub fn load_router(&self) -> anyhow::Result<RouterConfig> {
        match std::fs::read(self.router_path()) {
            Ok(data) => Ok(serde_json::from_slice(&data)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RouterConfig::default()),
            Err(e) => Err(e.into()),
        }
    }
}
