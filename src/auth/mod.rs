pub mod api_key;
pub mod oauth;
pub mod session;
pub mod token_store;

use anyhow::{Context, Result};

use crate::auth::session::AuthSession;

/// Manages authentication strategy selection and execution.
pub struct AuthManager {
    /// API key from --api-key flag or GPS_TRUST_API_KEY env var.
    api_key: Option<String>,
    /// Whether OAuth 2.1 is enabled (default: true).
    oauth: bool,
    /// User MCP server URL.
    user_url: String,
    /// Agent MCP server URL.
    agent_url: String,
}

impl AuthManager {
    pub fn new(
        api_key: Option<String>,
        oauth: bool,
        user_url: String,
        agent_url: String,
    ) -> Self {
        Self {
            api_key,
            oauth,
            user_url,
            agent_url,
        }
    }

    /// Build credentials. No MCP connections — identity is resolved later
    /// via McpManager::bootstrap_identity().
    ///
    /// - API key provided → use it (no network needed)
    /// - No API key + OAuth → OAuth flow (browser login)
    /// - Neither → error
    pub async fn authenticate(&self) -> Result<AuthSession> {
        if let Some(ref key) = self.api_key {
            return api_key::authenticate(key);
        }

        if self.oauth {
            return oauth::authenticate(&self.user_url, &self.agent_url)
                .await
                .context("OAuth authentication failed");
        }

        anyhow::bail!(
            "No authentication method available. \
             Use --api-key <key> or run without --no-oauth for OAuth login"
        )
    }
}
