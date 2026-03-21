use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::info;
use turul_mcp_client::config::{ClientConfig, ConnectionConfig};
use turul_mcp_client::McpClientBuilder;
use turul_mcp_protocol::ContentBlock;

use crate::auth::session::{AuthSession, ServerCredentials};
use crate::mcp::types::ServerIdentity;

/// Authenticate using an API key by connecting to the User MCP server
/// and calling `entity_info` to resolve account details.
pub async fn authenticate(api_key: &str, user_url: &str) -> Result<AuthSession> {
    info!("Authenticating with API key via entity_info");

    // Build MCP client with API key header
    let mut headers = HashMap::new();
    headers.insert("X-API-Key".to_string(), api_key.to_string());
    let config = ClientConfig {
        connection: ConnectionConfig {
            headers: Some(headers),
            ..Default::default()
        },
        ..Default::default()
    };
    let client = McpClientBuilder::new()
        .with_url(user_url)
        .context("invalid User MCP server URL")?
        .with_config(config)
        .build();

    client
        .connect()
        .await
        .context("failed to connect to User MCP server for auth")?;

    // Call entity_info — server infers identity from API key
    let results = client
        .call_tool("entity_info", serde_json::json!({}))
        .await
        .context("entity_info tool call failed")?;

    // Extract text from first content block
    let text = results
        .content
        .into_iter()
        .find_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text),
            _ => None,
        })
        .context("entity_info returned no text content")?;

    let parsed: serde_json::Value =
        serde_json::from_str(&text).context("entity_info returned invalid JSON")?;

    let account_id = parsed["accountId"]
        .as_str()
        .context("missing accountId in entity_info response")?
        .to_string();
    let display_name = parsed["entityName"]
        .as_str()
        .unwrap_or("Unknown")
        .to_string();
    let entity_type = parsed["entityType"]
        .as_str()
        .unwrap_or("account")
        .to_string();

    // Clean up the bootstrap client
    let _ = client.disconnect().await;

    info!(account_id = %account_id, display_name = %display_name, "API key auth successful");

    // Build credentials — same API key for both servers
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
        account_id,
        display_name,
        entity_type,
        credentials,
    })
}
