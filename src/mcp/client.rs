use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info, warn};
use turul_mcp_client::config::{ClientConfig, ConnectionConfig};
use turul_mcp_client::{McpClient, McpClientBuilder, ToolCallResponse};
use turul_mcp_protocol::{CallToolResult, Task, Tool};

use crate::action::Action;
use crate::mcp::notifications::dispatch_notification;
use crate::mcp::types::{ServerCaps, ServerIdentity};

/// Single-server MCP client wrapper.
pub struct McpServerClient {
    identity: ServerIdentity,
    client: McpClient,
    caps: ServerCaps,
}

impl McpServerClient {
    /// Build an MCP client for the given server with auth headers.
    pub fn new(
        identity: ServerIdentity,
        url: &str,
        headers: HashMap<String, String>,
    ) -> Result<Self> {
        let config = ClientConfig {
            connection: ConnectionConfig {
                headers: Some(headers),
                ..Default::default()
            },
            ..Default::default()
        };

        let client = McpClientBuilder::new()
            .with_url(url)
            .context("invalid MCP server URL")?
            .with_config(config)
            .build();

        Ok(Self {
            identity,
            client,
            caps: ServerCaps::default(),
        })
    }

    /// Connect to the server, inspect capabilities.
    pub async fn connect(&mut self) -> Result<()> {
        self.client
            .connect()
            .await
            .context("MCP connect failed")?;

        let info = self.client.session_info().await;
        self.caps = inspect_caps(&info);

        info!(
            server = %self.identity,
            tools_list_changed = self.caps.tools_list_changed,
            tasks_tool_call = self.caps.tasks_tool_call,
            tasks_cancel = self.caps.tasks_cancel,
            tasks_list = self.caps.tasks_list,
            "MCP server connected"
        );

        Ok(())
    }

    /// Disconnect from the server.
    pub async fn disconnect(&self) -> Result<()> {
        self.client
            .disconnect()
            .await
            .context("MCP disconnect failed")?;
        info!(server = %self.identity, "MCP server disconnected");
        Ok(())
    }

    /// Server capabilities detected after connect.
    #[allow(dead_code)]
    pub fn caps(&self) -> &ServerCaps {
        &self.caps
    }

    /// Server identity.
    #[allow(dead_code)]
    pub fn identity(&self) -> ServerIdentity {
        self.identity
    }

    /// List all tools using exhaustive pagination.
    pub async fn list_all_tools(&self) -> Result<Vec<Tool>> {
        let mut all_tools = Vec::new();
        let mut cursor = None;

        loop {
            let result = self
                .client
                .list_tools_paginated(cursor)
                .await
                .context("list_tools_paginated failed")?;

            all_tools.extend(result.tools);

            match result.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        debug!(
            server = %self.identity,
            count = all_tools.len(),
            "Listed all tools"
        );
        Ok(all_tools)
    }

    /// Synchronous tool call (no task augmentation).
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<CallToolResult> {
        self.client
            .call_tool(name, args)
            .await
            .context("call_tool failed")
    }

    /// Task-augmented tool call.
    #[allow(dead_code)]
    pub async fn call_tool_with_task(
        &self,
        name: &str,
        args: Value,
    ) -> Result<ToolCallResponse> {
        self.client
            .call_tool_with_task(name, args, None)
            .await
            .context("call_tool_with_task failed")
    }

    /// Get a task by ID.
    #[allow(dead_code)]
    pub async fn get_task(&self, task_id: &str) -> Result<Task> {
        self.client
            .get_task(task_id)
            .await
            .context("get_task failed")
    }

    /// Cancel a task by ID.
    #[allow(dead_code)]
    pub async fn cancel_task(&self, task_id: &str) -> Result<Task> {
        self.client
            .cancel_task(task_id)
            .await
            .context("cancel_task failed")
    }

    /// Access the underlying MCP client (for operations not wrapped here).
    #[allow(dead_code)]
    pub fn client_ref(&self) -> &McpClient {
        &self.client
    }

    /// Register notification and connection-lost callbacks that forward to the action channel.
    pub async fn setup_notifications(&self, tx: UnboundedSender<Action>) {
        let handler = self.client.stream_handler().await;
        let identity = self.identity;

        let notif_tx = tx.clone();
        handler.on_notification(move |value: Value| {
            dispatch_notification(identity, &value, &notif_tx);
        });

        let lost_tx = tx.clone();
        handler.on_connection_lost(move || {
            let _ = lost_tx.send(Action::McpDisconnected(identity));
        });

        let err_tx = tx;
        handler.on_error(move |msg: String| {
            warn!(server = %identity, error = %msg, "MCP stream error");
            let _ = err_tx.send(Action::McpError(identity, msg));
        });
    }
}

/// Inspect server capabilities from session info.
fn inspect_caps(info: &turul_mcp_client::session::SessionInfo) -> ServerCaps {
    let caps = info.server_capabilities.as_ref();
    ServerCaps {
        tools_list_changed: caps
            .and_then(|c| c.tools.as_ref())
            .and_then(|t| t.list_changed)
            .unwrap_or(false),
        tasks_tool_call: caps
            .and_then(|c| c.tasks.as_ref())
            .and_then(|t| t.requests.as_ref())
            .and_then(|r| r.tools.as_ref())
            .and_then(|t| t.call.as_ref())
            .is_some(),
        tasks_cancel: caps
            .and_then(|c| c.tasks.as_ref())
            .and_then(|t| t.cancel.as_ref())
            .is_some(),
        tasks_list: caps
            .and_then(|c| c.tasks.as_ref())
            .and_then(|t| t.list.as_ref())
            .is_some(),
    }
}
