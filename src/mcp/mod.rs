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
use crate::mcp::types::{
    ManagedFieldsPolicy, ServerId, ServerRegistry, ToolCallRequest, ToolEntry,
};

/// Identity resolved from entity_info on the connected identity-provider session.
pub struct IdentityInfo {
    pub account_id: String,
    pub display_name: String,
    pub entity_type: String,
}

/// Orchestrates connections to all configured MCP servers.
pub struct McpManager {
    clients: HashMap<ServerId, McpServerClient>,
    registry: ServerRegistry,
    managed_fields: ManagedFieldsPolicy,
    // Per-server headers, kept for reconnect (clients are rebuilt from scratch).
    headers: HashMap<ServerId, HashMap<String, String>>,
}

impl McpManager {
    /// Create the manager with one client per registered server, configured
    /// from the auth session. ManagedFieldsPolicy starts from the session's
    /// account_id (OAuth has it from the JWT); bootstrap_identity() refines it.
    pub fn new(session: &AuthSession, registry: &ServerRegistry) -> Result<Self> {
        let mut clients = HashMap::new();
        let mut headers = HashMap::new();

        for server in registry.iter() {
            let server_headers = session.headers_for(server);
            let client =
                McpServerClient::new(server.clone(), server.url(), server_headers.clone())
                    .with_context(|| format!("failed to create {server} MCP client"))?;
            clients.insert(server.clone(), client);
            headers.insert(server.clone(), server_headers);
        }

        let managed_fields = if session.account_id.is_empty() {
            ManagedFieldsPolicy::new("")
        } else {
            ManagedFieldsPolicy::new(&session.account_id)
        };

        Ok(Self {
            clients,
            registry: registry.clone(),
            managed_fields,
            headers,
        })
    }

    /// Connect every server in registry order and set up notification forwarding.
    pub async fn connect_all(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        let servers: Vec<ServerId> = self.registry.iter().cloned().collect();
        for server in &servers {
            let client = self
                .clients
                .get_mut(server)
                .expect("client exists for every registered server");

            let _ = tx.send(Action::McpConnecting(server.clone()));
            client
                .connect()
                .await
                .with_context(|| format!("{server} MCP connect failed"))?;
            let _ = tx.send(Action::McpConnected(server.clone()));
            client.setup_notifications(tx.clone()).await;
        }

        info!(count = servers.len(), "All MCP servers connected");
        Ok(())
    }

    /// Call entity_info on the connected identity-provider client to resolve
    /// identity. Updates managed_fields with the real account_id. Call this
    /// after connect_all().
    pub async fn bootstrap_identity(&mut self) -> Result<IdentityInfo> {
        let provider = self
            .registry
            .identity_provider()
            .cloned()
            .context("no identity-provider server configured")?;
        let client = self
            .clients
            .get(&provider)
            .context("identity-provider client missing")?;

        let result = client
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

    /// Rebuild all clients and reconnect. Used when connections drop.
    pub async fn reconnect_all(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        for client in self.clients.values() {
            let _ = client.disconnect().await;
        }

        let servers: Vec<ServerId> = self.registry.iter().cloned().collect();
        let mut clients = HashMap::new();
        for server in &servers {
            let server_headers = self.headers.get(server).cloned().unwrap_or_default();
            let client = McpServerClient::new(server.clone(), server.url(), server_headers)
                .with_context(|| format!("failed to rebuild {server} MCP client"))?;
            clients.insert(server.clone(), client);
        }
        self.clients = clients;

        self.connect_all(tx).await
    }

    /// List all tools from every server, tagged by origin.
    pub async fn list_all_tools(&self) -> Result<Vec<ToolEntry>> {
        let mut entries: Vec<ToolEntry> = Vec::new();
        for server in self.registry.iter() {
            let client = self
                .clients
                .get(server)
                .expect("client exists for every registered server");
            let tools = client
                .list_all_tools()
                .await
                .with_context(|| format!("{server} list_all_tools failed"))?;
            for tool in tools {
                entries.push(ToolEntry {
                    server: server.clone(),
                    tool,
                });
            }
        }

        debug!(count = entries.len(), "Merged tool list from all servers");
        Ok(entries)
    }

    /// Call a tool, routing to the correct server.
    pub async fn call_tool(&self, mut request: ToolCallRequest) -> Result<CallToolResult> {
        self.managed_fields
            .inject(&mut request.arguments)
            .map_err(|e| anyhow::anyhow!(e))?;

        let client = self
            .clients
            .get(&request.server)
            .with_context(|| format!("no client for server {}", request.server))?;
        client
            .call_tool(&request.tool_name, request.arguments)
            .await
    }

    /// Disconnect every server.
    pub async fn disconnect_all(&self) -> Result<()> {
        for client in self.clients.values() {
            client.disconnect().await?;
        }
        Ok(())
    }
}
