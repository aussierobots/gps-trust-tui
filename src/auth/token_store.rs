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
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context("failed to create config directory for token store")?;
        }
        let json =
            serde_json::to_string_pretty(self).context("failed to serialize token store")?;
        std::fs::write(&path, &json).context("failed to write token store")?;

        // Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&path, perms)
                .context("failed to set token store permissions")?;
        }

        Ok(())
    }
}
