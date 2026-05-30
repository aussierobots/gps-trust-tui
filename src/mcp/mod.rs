pub mod client;
pub mod notifications;
pub mod types;

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info, warn};
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
    // Registered servers with no credential — never connected, surfaced as Unauthorized.
    unauthorized: Vec<ServerId>,
    // Servers currently connected (updated by connect_all).
    connected: HashSet<ServerId>,
}

impl McpManager {
    /// Create the manager with one client per *credentialed* server, configured
    /// from the auth session. Servers without a credential are recorded as
    /// unauthorized and never connected (fail-closed). ManagedFieldsPolicy
    /// starts from the session's account_id (OAuth has it from the JWT);
    /// bootstrap_identity() refines it.
    pub fn new(session: &AuthSession, registry: &ServerRegistry) -> Result<Self> {
        let mut clients = HashMap::new();
        let mut headers = HashMap::new();
        let mut unauthorized = Vec::new();

        for server in registry.iter() {
            // Fail-closed: no *usable* credential (missing, or an OAuth token
            // whose audience doesn't match this server) → no client.
            if !session.has_usable_credential(server) {
                unauthorized.push(server.clone());
                continue;
            }
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
            unauthorized,
            connected: HashSet::new(),
        })
    }

    /// Connect every credentialed server. Fail-soft: a single server's failure
    /// marks it errored and continues; only a total failure (zero connected)
    /// aborts. Servers without a credential are surfaced as Unauthorized.
    pub async fn connect_all(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        self.connected.clear();

        for server in &self.unauthorized {
            warn!(server = %server, "no credential — marking server unauthorized");
            let _ = tx.send(Action::McpUnauthorized(server.clone()));
        }

        let servers: Vec<ServerId> = self
            .registry
            .iter()
            .filter(|s| self.clients.contains_key(*s))
            .cloned()
            .collect();

        for server in &servers {
            let client = self
                .clients
                .get_mut(server)
                .expect("client exists for every credentialed server");

            let _ = tx.send(Action::McpConnecting(server.clone()));
            match client.connect().await {
                Ok(()) => {
                    let _ = tx.send(Action::McpConnected(server.clone()));
                    client.setup_notifications(tx.clone()).await;
                    self.connected.insert(server.clone());
                }
                Err(e) => {
                    // Fail-soft: one server's failure must not abort the others.
                    warn!(server = %server, error = %e, "MCP connect failed — continuing");
                    let _ = tx.send(Action::McpError(server.clone(), e.to_string()));
                }
            }
        }

        if self.connected.is_empty() {
            bail!("no MCP servers connected");
        }
        info!(
            connected = self.connected.len(),
            total = servers.len(),
            "MCP connect complete"
        );
        Ok(())
    }

    /// Call entity_info on the connected identity-provider client to resolve
    /// identity. Updates managed_fields with the real account_id. Call this
    /// after connect_all(). Errors (soft) if the identity provider is not
    /// connected — the caller falls back to the token's account_id.
    pub async fn bootstrap_identity(&mut self) -> Result<IdentityInfo> {
        let provider = self
            .registry
            .identity_provider()
            .cloned()
            .context("no identity-provider server configured")?;
        if !self.connected.contains(&provider) {
            bail!("identity-provider server {provider} is not connected");
        }
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

    /// Rebuild all credentialed clients and reconnect. Used when connections drop.
    pub async fn reconnect_all(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        for client in self.clients.values() {
            let _ = client.disconnect().await;
        }

        let mut clients = HashMap::new();
        for server in self.registry.iter() {
            // Only rebuild servers we hold credentials (headers) for.
            let Some(server_headers) = self.headers.get(server).cloned() else {
                continue;
            };
            let client = McpServerClient::new(server.clone(), server.url(), server_headers)
                .with_context(|| format!("failed to rebuild {server} MCP client"))?;
            clients.insert(server.clone(), client);
        }
        self.clients = clients;

        self.connect_all(tx).await
    }

    /// List all tools from every *connected* server, tagged by origin. Fail-soft:
    /// a server whose listing fails is logged and skipped.
    pub async fn list_all_tools(&self) -> Result<Vec<ToolEntry>> {
        let mut entries: Vec<ToolEntry> = Vec::new();
        for server in self.registry.iter() {
            if !self.connected.contains(server) {
                continue;
            }
            let client = self
                .clients
                .get(server)
                .expect("connected client exists");
            match client.list_all_tools().await {
                Ok(tools) => {
                    for tool in tools {
                        entries.push(ToolEntry {
                            server: server.clone(),
                            tool,
                        });
                    }
                }
                Err(e) => {
                    warn!(server = %server, error = %e, "list_all_tools failed — skipping server");
                }
            }
        }

        debug!(count = entries.len(), "Merged tool list from connected servers");
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
            .with_context(|| format!("server {} is not available", request.server))?;
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
