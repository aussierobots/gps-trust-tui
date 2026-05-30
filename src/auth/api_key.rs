use std::collections::HashMap;

use anyhow::Result;
use tracing::info;

use crate::auth::session::{AuthSession, ServerCredentials};
use crate::mcp::types::ServerRegistry;

/// Build an AuthSession from an API key. No MCP connection needed —
/// identity is resolved later via McpManager::bootstrap_identity().
///
/// The same key is presented to every configured server; each server's Lambda
/// authorizer validates it independently.
pub fn authenticate(api_key: &str, registry: &ServerRegistry) -> Result<AuthSession> {
    info!("Using API key authentication");

    let mut credentials = HashMap::new();
    for server in registry.iter() {
        credentials.insert(
            server.clone(),
            ServerCredentials::ApiKey {
                key: api_key.to_string(),
            },
        );
    }

    Ok(AuthSession {
        account_id: String::new(), // Resolved after MCP connect
        display_name: String::new(),
        entity_type: String::new(),
        credentials,
    })
}
