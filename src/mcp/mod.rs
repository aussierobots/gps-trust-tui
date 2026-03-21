pub mod client;
pub mod notifications;
pub mod types;

use anyhow::{Context, Result};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info};
use turul_mcp_protocol::CallToolResult;

use crate::action::Action;
use crate::auth::session::AuthSession;
use crate::mcp::client::McpServerClient;
use crate::mcp::types::{ManagedFieldsPolicy, ServerIdentity, ToolCallRequest, ToolEntry};

/// Orchestrates connections to both User and Agent MCP servers.
pub struct McpManager {
    user_client: McpServerClient,
    agent_client: McpServerClient,
    managed_fields: ManagedFieldsPolicy,
}

impl McpManager {
    /// Create the manager with both clients configured from the auth session.
    pub fn new(session: &AuthSession, user_url: &str, agent_url: &str) -> Result<Self> {
        let user_headers = session.headers_for(&ServerIdentity::User);
        let agent_headers = session.headers_for(&ServerIdentity::Agent);

        let user_client =
            McpServerClient::new(ServerIdentity::User, user_url, user_headers)
                .context("failed to create User MCP client")?;
        let agent_client =
            McpServerClient::new(ServerIdentity::Agent, agent_url, agent_headers)
                .context("failed to create Agent MCP client")?;

        let managed_fields = ManagedFieldsPolicy::new(&session.account_id);

        Ok(Self {
            user_client,
            agent_client,
            managed_fields,
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
    ///
    /// Managed fields are injected before dispatch.
    pub async fn call_tool(&self, mut request: ToolCallRequest) -> Result<CallToolResult> {
        // Inject managed fields (e.g. account_id)
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
        // Best-effort disconnect both; report first error
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
