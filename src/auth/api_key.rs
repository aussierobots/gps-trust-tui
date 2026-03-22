use std::collections::HashMap;

use anyhow::Result;
use tracing::info;

use crate::auth::session::{AuthSession, ServerCredentials};
use crate::mcp::types::ServerIdentity;

/// Build an AuthSession from an API key. No MCP connection needed —
/// identity is resolved later via McpManager::bootstrap_identity().
pub fn authenticate(api_key: &str) -> Result<AuthSession> {
    info!("Using API key authentication");

    let mut credentials = HashMap::new();
    credentials.insert(
        ServerIdentity::User,
        ServerCredentials::ApiKey {
            key: api_key.to_string(),
        },
    );
    credentials.insert(
        ServerIdentity::Agent,
        ServerCredentials::ApiKey {
            key: api_key.to_string(),
        },
    );

    Ok(AuthSession {
        account_id: String::new(), // Resolved after MCP connect
        display_name: String::new(),
        entity_type: String::new(),
        credentials,
    })
}
