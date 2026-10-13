use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenStore {
    /// DCR client_id, cached after dynamic registration
    pub dcr_client_id: Option<String>,
    /// Audience-keyed token entries
    pub tokens: HashMap<String, StoredToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub refresh_token: String,
    pub expires_at: i64,
    pub account_id: String,
}

impl TokenStore {
    /// Path to the token store file.
    fn path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir().context("cannot determine config directory")?;
        Ok(config_dir.join("gps-trust").join("tokens.json"))
    }

    /// Load the token store from disk. Returns an empty store if the file
    /// does not exist or cannot be parsed.
    pub fn load() -> TokenStore {
        let Ok(path) = Self::path() else {
            return TokenStore::default();
        };
        let Ok(data) = std::fs::read_to_string(&path) else {
            return TokenStore::default();
        };
        serde_json::from_str(&data).unwrap_or_default()
    }

    /// Persist the token store to disk with 0600 permissions.
    ///
    /// On unix, the file is opened with mode 0600 set *at creation* (not
    /// chmod'd afterward) so a freshly-created store is never briefly
    /// world/group-readable at the process umask before being tightened.
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context("failed to create config directory for token store")?;
        }
        let json =
            serde_json::to_string_pretty(self).context("failed to serialize token store")?;

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)
                .context("failed to open token store for writing")?;
            file.write_all(json.as_bytes())
                .context("failed to write token store")?;
            // mode() only governs creation; an existing file keeps its prior
            // permissions, so tighten explicitly for that case too.
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&path, perms)
                .context("failed to set token store permissions")?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&path, &json).context("failed to write token store")?;
        }

        Ok(())
    }

    /// Delete the token store file from disk.
    pub fn delete() -> Result<()> {
        let path = Self::path()?;
        if path.exists() {
            std::fs::remove_file(&path).context("failed to delete token store")?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Regression test for the world/group-readable window: with a
    /// permissive umask, a freshly-created token store must still land at
    /// 0600 (not the umask-derived mode `std::fs::write` alone would give).
    #[test]
    fn save_creates_file_at_0600_under_permissive_umask() {
        let dir = gt_data_layout::tmp_dir().join(format!(
            "gttui-token-store-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: test-only; no other thread in this process touches these
        // env vars, and each test uses a unique temp dir.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
        }

        // A permissive umask is exactly the condition that used to leak
        // plaintext refresh tokens via std::fs::write's default mode.
        let old_umask = unsafe { libc_umask(0o022) };

        let store = TokenStore {
            dcr_client_id: None,
            tokens: HashMap::new(),
        };
        store.save().expect("save should succeed");

        let path = dir.join("gps-trust").join("tokens.json");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;

        unsafe {
            libc_umask(old_umask);
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(mode, 0o600, "token store must be 0600, got {mode:o}");
    }

    // Minimal umask(2) binding — avoids pulling in the `libc` crate for one
    // syscall used only by this test.
    unsafe extern "C" {
        #[link_name = "umask"]
        fn libc_umask(mask: u32) -> u32;
    }
}
