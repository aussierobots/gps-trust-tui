pub mod api_key;
pub mod oauth;
pub mod session;
pub mod token_store;

use anyhow::{Context, Result};
use tracing::info;

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

    /// Authenticate using the configured strategy:
    ///
    /// - `--api-key` + `--oauth` (default): OAuth for identity, API key as fallback
    /// - `--api-key` + `--no-oauth`: API key only for both servers
    /// - `--oauth` without API key: OAuth only
    /// - Neither: error
    pub async fn authenticate(&self) -> Result<AuthSession> {
        match (self.api_key.as_ref(), self.oauth) {
            // OAuth enabled (with or without API key)
            (_, true) => {
                info!("Using OAuth 2.1 authentication");
                let session = oauth::authenticate(&self.user_url, &self.agent_url).await;

                match (session, self.api_key.as_ref()) {
                    (Ok(session), _) => Ok(session),
                    (Err(e), Some(key)) => {
                        // OAuth failed but we have an API key — fall back
                        info!(
                            error = %e,
                            "OAuth failed, falling back to API key authentication"
                        );
                        api_key::authenticate(key, &self.user_url).await
                    }
                    (Err(e), None) => Err(e).context("OAuth authentication failed"),
                }
            }

            // API key only (--no-oauth)
            (Some(key), false) => {
                info!("Using API key authentication");
                api_key::authenticate(key, &self.user_url).await
            }

            // No auth method available
            (None, false) => {
                anyhow::bail!(
                    "No authentication method available. \
                     Use --api-key <key> or --oauth (default)"
                )
            }
        }
    }
}
