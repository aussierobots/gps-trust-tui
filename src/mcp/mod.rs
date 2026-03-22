pub mod client;
pub mod notifications;
pub mod types;

use std::collections::HashMap;

use anyhow::{Context, Result};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info};
use turul_mcp_protocol::{CallToolResult, ContentBlock};

use crate::action::Action;
use crate::auth::session::AuthSession;
use crate::mcp::client::McpServerClient;
use crate::mcp::types::{ManagedFieldsPolicy, ServerIdentity, ToolCallRequest, ToolEntry};

/// Identity resolved from entity_info on the connected User MCP session.
pub struct IdentityInfo {
    pub account_id: String,
    pub display_name: String,
    pub entity_type: String,
}

/// Orchestrates connections to both User and Agent MCP servers.
pub struct McpManager {
    user_client: McpServerClient,
    agent_client: McpServerClient,
    managed_fields: ManagedFieldsPolicy,
    // Stored for reconnect — clients need to be rebuilt from scratch
    user_url: String,
    agent_url: String,
    user_headers: HashMap<String, String>,
    agent_headers: HashMap<String, String>,
}

impl McpManager {
    /// Create the manager with both clients configured from the auth session.
    /// ManagedFieldsPolicy starts empty — call bootstrap_identity() after connect.
    pub fn new(session: &AuthSession, user_url: &str, agent_url: &str) -> Result<Self> {
        let user_headers = session.headers_for(&ServerIdentity::User);
        let agent_headers = session.headers_for(&ServerIdentity::Agent);

        let user_client =
            McpServerClient::new(ServerIdentity::User, user_url, user_headers.clone())
                .context("failed to create User MCP client")?;
        let agent_client =
            McpServerClient::new(ServerIdentity::Agent, agent_url, agent_headers.clone())
                .context("failed to create Agent MCP client")?;

        // Start with account_id from session if available (OAuth has it from JWT),
        // otherwise empty — bootstrap_identity() will fill it in.
        let managed_fields = if session.account_id.is_empty() {
            ManagedFieldsPolicy::new("")
        } else {
            ManagedFieldsPolicy::new(&session.account_id)
        };

        Ok(Self {
            user_client,
            agent_client,
            managed_fields,
            user_url: user_url.to_string(),
            agent_url: agent_url.to_string(),
            user_headers,
            agent_headers,
        })
    }

    /// Connect both servers and set up notification forwarding.
    pub async fn connect_all(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        let _ = tx.send(Action::McpConnecting(ServerIdentity::User));
        self.user_client
            .connect()
            .await
            .context("User MCP connect failed")?;
        let _ = tx.send(Action::McpConnected(ServerIdentity::User));
        self.user_client.setup_notifications(tx.clone()).await;

        let _ = tx.send(Action::McpConnecting(ServerIdentity::Agent));
        self.agent_client
            .connect()
            .await
            .context("Agent MCP connect failed")?;
        let _ = tx.send(Action::McpConnected(ServerIdentity::Agent));
        self.agent_client.setup_notifications(tx).await;

        info!("Both MCP servers connected");
        Ok(())
    }

    /// Call entity_info on the already-connected User client to resolve identity.
    /// Updates managed_fields with the real account_id.
    /// Call this after connect_all().
    pub async fn bootstrap_identity(&mut self) -> Result<IdentityInfo> {
        let result = self
            .user_client
            .call_tool("entity_info", serde_json::json!({}))
            .await
            .context("entity_info bootstrap call failed")?;

        let text = result
            .content
            .into_iter()
            .find_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(text),
                _ => None,
            })
            .context("entity_info returned no text content")?;

        let parsed: serde_json::Value =
            serde_json::from_str(&text).context("entity_info returned invalid JSON")?;

        // Response may be wrapped: {"entityInfoOutput": {...}}
        let info = parsed.get("entityInfoOutput").unwrap_or(&parsed);

        let account_id = info["accountId"]
            .as_str()
            .context("missing accountId in entity_info response")?
            .to_string();
        let display_name = info["entityName"]
            .as_str()
            .unwrap_or("Unknown")
            .to_string();
        let entity_type = info["entityType"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        // Update managed fields with the real account_id
        self.managed_fields = ManagedFieldsPolicy::new(&account_id);

        info!(
            account_id = %account_id,
            display_name = %display_name,
            entity_type = %entity_type,
            "Identity bootstrapped from entity_info"
        );

        Ok(IdentityInfo {
            account_id,
            display_name,
            entity_type,
        })
    }

    /// Rebuild clients and reconnect. Used when connections drop.
    pub async fn reconnect_all(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        let _ = self.user_client.disconnect().await;
        let _ = self.agent_client.disconnect().await;

        self.user_client =
            McpServerClient::new(ServerIdentity::User, &self.user_url, self.user_headers.clone())
                .context("failed to rebuild User MCP client")?;
        self.agent_client =
            McpServerClient::new(ServerIdentity::Agent, &self.agent_url, self.agent_headers.clone())
                .context("failed to rebuild Agent MCP client")?;

        self.connect_all(tx).await
    }

    /// List all tools from both servers, tagged by origin.
    pub async fn list_all_tools(&self) -> Result<Vec<ToolEntry>> {
        let user_tools = self
            .user_client
            .list_all_tools()
            .await
            .context("User list_all_tools failed")?;
        let agent_tools = self
            .agent_client
            .list_all_tools()
            .await
            .context("Agent list_all_tools failed")?;

        let mut entries: Vec<ToolEntry> = Vec::with_capacity(user_tools.len() + agent_tools.len());
        for tool in user_tools {
            entries.push(ToolEntry {
                server: ServerIdentity::User,
                tool,
            });
        }
        for tool in agent_tools {
            entries.push(ToolEntry {
                server: ServerIdentity::Agent,
                tool,
            });
        }

        debug!(count = entries.len(), "Merged tool list from both servers");
        Ok(entries)
    }

    /// Call a tool, routing to the correct server.
    pub async fn call_tool(&self, mut request: ToolCallRequest) -> Result<CallToolResult> {
        self.managed_fields
            .inject(&mut request.arguments)
            .map_err(|e| anyhow::anyhow!(e))?;

        let client = self.client_for(request.server);
        client
            .call_tool(&request.tool_name, request.arguments)
            .await
    }

    /// Disconnect both servers.
    pub async fn disconnect_all(&self) -> Result<()> {
        let r1 = self.user_client.disconnect().await;
        let r2 = self.agent_client.disconnect().await;
        r1?;
        r2?;
        Ok(())
    }

    fn client_for(&self, server: ServerIdentity) -> &McpServerClient {
        match server {
            ServerIdentity::User => &self.user_client,
            ServerIdentity::Agent => &self.agent_client,
        }
    }
}
