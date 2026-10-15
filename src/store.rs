use std::path::PathBuf;
use std::sync::Mutex;

use crate::account::AccountData;

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

    pub fn save(&self, accounts: &[AccountData]) -> anyhow::Result<()> {
        let _guard = self.mu.lock().expect("store lock poisoned");
        let data = serde_json::to_vec_pretty(accounts)?;
        if let Some(dir) = self.path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn load(&self) -> anyhow::Result<Vec<AccountData>> {
        let _guard = self.mu.lock().expect("store lock poisoned");
        match std::fs::read(&self.path) {
            Ok(data) => Ok(serde_json::from_slice(&data)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }
}
