use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use turul_mcp_client::config::{ClientConfig, ConnectionConfig};
use turul_mcp_client::{CallToolResult, Tool};
use turul_mcp_client::{McpClient, McpClientBuilder, McpVersion};

use crate::action::Action;
use crate::mcp::notifications::dispatch_notification;
use crate::mcp::types::{ServerCaps, ServerId};

/// Single-server MCP client wrapper.
pub struct McpServerClient {
    identity: ServerId,
    client: Arc<McpClient>,
    caps: ServerCaps,
    /// Wire spec negotiated at connect; `None` before the first connect.
    lane: Option<McpVersion>,
    /// Action channel, set by `setup_notifications`; 2026 progress rides it.
    tx: OnceLock<UnboundedSender<Action>>,
    /// The 2026 `subscriptions/listen` task, aborted on disconnect.
    listen: Mutex<Option<JoinHandle<()>>>,
    next_progress_token: AtomicU64,
}

/// Pause before reopening a 2026 listen stream that ended or failed to open.
const LISTEN_RETRY: Duration = Duration::from_secs(5);

impl McpServerClient {
    /// Build an MCP client for the given server with auth headers.
    pub fn new(
        identity: ServerId,
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
            client: Arc::new(client),
            caps: ServerCaps::default(),
            lane: None,
            tx: OnceLock::new(),
            listen: Mutex::new(None),
            next_progress_token: AtomicU64::new(0),
        })
    }

    /// Connect to the server, inspect capabilities.
    pub async fn connect(&mut self) -> Result<()> {
        self.client
            .connect()
            .await
            .context("MCP connect failed")?;

        // Bilingual client (turul-mcp-client 0.4): one wire spec per connection,
        // "2025-11-25" or "2026-07-28" depending on what the server speaks.
        let lane = self.client.negotiated_version().await;
        self.lane = lane;
        self.caps = self.inspect_caps(lane).await;

        info!(
            server = %self.identity,
            protocol_version = lane.map(|v| v.as_str()).unwrap_or("unknown"),
            tools_list_changed = self.caps.tools_list_changed,
            "MCP server connected"
        );

        Ok(())
    }

    /// Disconnect from the server.
    pub async fn disconnect(&self) -> Result<()> {
        if let Some(handle) = self.listen.lock().expect("listen lock").take() {
            handle.abort();
        }
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
    pub fn identity(&self) -> ServerId {
        self.identity.clone()
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

    /// Tool call. On a 2026-07-28 lane the call carries a progress token and
    /// its progress rides the request stream into the action channel; on
    /// 2025-11-25 progress arrives on the GET stream handler instead.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<CallToolResult> {
        if self.lane == Some(McpVersion::V2026_07_28)
            && let Some(tx) = self.tx.get()
        {
            let token = format!(
                "gttui-{}",
                self.next_progress_token.fetch_add(1, Ordering::Relaxed)
            );
            let server = self.identity.clone();
            let tx = tx.clone();
            return self
                .client
                .call_tool_with_progress(name, args, json!(token), move |params| {
                    let notification = json!({"method": "notifications/progress", "params": params});
                    dispatch_notification(&server, &notification, &tx);
                })
                .await
                .context("call_tool failed");
        }
        self.client
            .call_tool(name, args)
            .await
            .context("call_tool failed")
    }

    /// Access the underlying MCP client (for operations not wrapped here).
    #[allow(dead_code)]
    pub fn client_ref(&self) -> &McpClient {
        &self.client
    }

    /// Register notification and connection-lost callbacks that forward to the action channel.
    pub async fn setup_notifications(&self, tx: UnboundedSender<Action>) {
        let _ = self.tx.set(tx.clone());
        if self.lane == Some(McpVersion::V2026_07_28) {
            self.spawn_listen(tx);
            return;
        }
        let handler = self.client.stream_handler().await;

        let notif_identity = self.identity.clone();
        let notif_tx = tx.clone();
        handler.on_notification(move |value: Value| {
            dispatch_notification(&notif_identity, &value, &notif_tx);
        });

        let lost_identity = self.identity.clone();
        handler.on_connection_lost(move || {
            // Do NOT mark the server Disconnected here. This fires whenever the
            // best-effort SSE push channel (server->client notifications) ends,
            // which is routine — not a sign the server is unreachable — and on
            // this stack it is *structurally guaranteed* to happen roughly every
            // 30s: GPS Trust MCP servers are AWS Lambda behind API Gateway, which
            // has a hard ~29s integration timeout, so any long-held SSE GET gets
            // killed by the infra on that cadence regardless of client/server
            // behavior. Every tool call is an independent POST that doesn't
            // depend on this side-channel. Confirmed by driving a live session:
            // the status bar flipped to "disconnected" mid-use while tool calls
            // kept succeeding. Same rationale as `on_error` below — only surface
            // this in the trace log.
            warn!(server = %lost_identity, "MCP notification stream ended (tool calls unaffected)");
        });

        let err_identity = self.identity.clone();
        handler.on_error(move |msg: String| {
            // Log but don't set connection state to Error — stream errors are
            // expected on HTTP transport (no persistent event stream).
            // Connection state is only meaningful for SSE transports.
            warn!(server = %err_identity, error = %msg, "MCP stream error (non-fatal)");
        });
    }

    /// 2026-07-28 has no GET stream: hold a `subscriptions/listen` stream for
    /// `tools/list_changed` instead, reopening it whenever it ends (the same
    /// ~29s API Gateway cut-off noted above applies).
    fn spawn_listen(&self, tx: UnboundedSender<Action>) {
        if !self.caps.tools_list_changed {
            return;
        }
        let client = Arc::clone(&self.client);
        let identity = self.identity.clone();
        let handle = tokio::spawn(async move {
            loop {
                match client
                    .subscriptions_listen(json!({"toolsListChanged": true}))
                    .await
                {
                    Ok(mut stream) => {
                        while let Some(value) = stream.next().await {
                            dispatch_notification(&identity, &value, &tx);
                        }
                        warn!(server = %identity, "MCP listen stream ended (tool calls unaffected)");
                    }
                    Err(e) => {
                        warn!(server = %identity, error = %e, "MCP listen stream failed to open");
                    }
                }
                tokio::time::sleep(LISTEN_RETRY).await;
            }
        });
        *self.listen.lock().expect("listen lock") = Some(handle);
    }

    /// Capabilities for the negotiated lane. 2025-11-25 keeps them on the
    /// session; the stateless 2026-07-28 lane leaves the session empty and
    /// keeps the `server/discover` result instead.
    async fn inspect_caps(&self, lane: Option<McpVersion>) -> ServerCaps {
        let tools_list_changed = match lane {
            Some(McpVersion::V2026_07_28) => self
                .client
                .server_capabilities()
                .await
                .and_then(|c| c.pointer("/tools/listChanged").and_then(Value::as_bool))
                .unwrap_or(false),
            _ => self
                .client
                .session_info()
                .await
                .server_capabilities
                .and_then(|c| c.tools)
                .and_then(|t| t.list_changed)
                .unwrap_or(false),
        };
        ServerCaps { tools_list_changed }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
    use turul_mcp_derive::McpTool;
    use turul_mcp_server::prelude::{McpError, McpResult, McpServer};

    use super::*;
    use crate::mcp::types::ServerConfig;

    fn identity(url: &str) -> ServerId {
        ServerId::new(ServerConfig {
            key: "test".into(),
            label: "Test".into(),
            prefix: "T".into(),
            url: url.into(),
            is_identity_provider: false,
            scope: "mcp:read".into(),
        })
    }

    /// Emits one request-scoped progress notification, then completes.
    #[derive(McpTool, Clone, Default)]
    #[tool(name = "progress_worker", description = "Works with progress", output = String)]
    struct ProgressWorkerTool {}

    impl ProgressWorkerTool {
        async fn execute(
            &self,
            session: Option<turul_mcp_server::SessionContext>,
        ) -> McpResult<String> {
            if let Some(session) = session {
                session.notify_request_progress(0.5, Some(1.0)).await;
            }
            Ok("worked".to_string())
        }
    }

    /// Broadcasts `notifications/tools/list_changed` to every session.
    #[derive(McpTool, Clone, Default)]
    #[tool(name = "emit_tools_changed", description = "Broadcast a tools change", output = String)]
    struct EmitToolsChangedTool {}

    impl EmitToolsChangedTool {
        async fn execute(
            &self,
            session: Option<turul_mcp_server::SessionContext>,
        ) -> McpResult<String> {
            use turul_mcp_server::turul_http_mcp_server::notification_bridge::SharedNotificationBroadcaster;
            let session = session.ok_or_else(|| McpError::tool_execution("session required"))?;
            let broadcaster = session
                .broadcaster
                .as_ref()
                .and_then(|a| a.downcast_ref::<SharedNotificationBroadcaster>())
                .ok_or_else(|| McpError::tool_execution("broadcaster required"))?
                .clone();
            let _ = broadcaster
                .broadcast_to_all_sessions(turul_rpc::JsonRpcNotification::new_no_params(
                    "notifications/tools/list_changed".to_string(),
                ))
                .await;
            Ok("emitted".to_string())
        }
    }

    async fn wait_listening(url: &str) {
        let probe = reqwest::Client::new();
        for _ in 0..50 {
            if probe.get(url).send().await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("in-process server did not start at {url}");
    }

    /// In-process turul-mcp-server 0.4: speaks 2026-07-28 and answers `server/discover`.
    async fn start_2026_server() -> String {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let server = McpServer::builder()
            .name("gttui-2026")
            .version("0.4.0")
            .tool(ProgressWorkerTool::default())
            .tool(EmitToolsChangedTool::default())
            .tool_change_mode(turul_mcp_server::ToolChangeMode::Dynamic)
            .bind_address(format!("127.0.0.1:{port}").parse().unwrap())
            .build()
            .expect("build 2026 server");
        tokio::spawn(async move {
            server.run().await.ok();
        });
        let url = format!("http://127.0.0.1:{port}/mcp");
        wait_listening(&url).await;
        url
    }

    /// In-process turul-mcp-server 0.3.47: the stateful server the fleet runs today.
    async fn start_legacy_server() -> String {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let server = turul_mcp_server_legacy::McpServer::builder()
            .name("gttui-legacy")
            .version("0.3.47")
            .bind_address(format!("127.0.0.1:{port}").parse().unwrap())
            .sse(false)
            .build()
            .expect("build legacy server");
        tokio::spawn(async move { server.run().await });
        let url = format!("http://127.0.0.1:{port}/mcp");
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                return url;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("legacy server did not start");
    }

    async fn connected(url: &str) -> (McpServerClient, UnboundedReceiver<Action>) {
        let mut client = McpServerClient::new(identity(url), url, HashMap::new()).unwrap();
        client.connect().await.expect("connect with no version pin");
        let (tx, rx) = unbounded_channel();
        client.setup_notifications(tx).await;
        (client, rx)
    }

    #[tokio::test]
    async fn a_0_3_server_negotiates_the_2025_lane_with_no_pin() {
        let url = start_legacy_server().await;
        let (client, _rx) = connected(&url).await;
        assert_eq!(client.lane, Some(McpVersion::V2025_11_25));
        assert!(client.list_all_tools().await.expect("tools/list").is_empty());
        // The 2025 lane never takes the 2026-only progress path.
        let err = client
            .call_tool("missing", json!({}))
            .await
            .expect_err("no such tool");
        assert!(!format!("{err:#}").contains("requires a 2026-07-28"), "{err:#}");
    }

    #[tokio::test]
    async fn a_2026_server_negotiates_the_2026_lane_and_reads_caps_from_discover() {
        let url = start_2026_server().await;
        let (client, _rx) = connected(&url).await;
        assert_eq!(client.lane, Some(McpVersion::V2026_07_28));
        let declared = client
            .client
            .server_capabilities()
            .await
            .and_then(|c| c.pointer("/tools/listChanged").and_then(Value::as_bool))
            .unwrap_or(false);
        assert_eq!(client.caps().tools_list_changed, declared);
        assert!(declared, "the 0.4 server declares tools.listChanged");
    }

    #[tokio::test]
    async fn a_2026_call_forwards_progress_to_the_action_channel() {
        let url = start_2026_server().await;
        let (client, mut rx) = connected(&url).await;
        let result = client
            .call_tool("progress_worker", json!({}))
            .await
            .expect("call");
        assert!(serde_json::to_string(&result).unwrap().contains("worked"));
        match rx.try_recv().expect("a progress action") {
            Action::McpProgress {
                progress, total, ..
            } => {
                assert_eq!(progress, 0.5);
                assert_eq!(total, Some(1.0));
            }
            other => panic!("expected McpProgress, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_2026_listen_stream_forwards_tools_list_changed() {
        let url = start_2026_server().await;
        let (client, mut rx) = connected(&url).await;
        // Give the spawned listen task time to open and be acknowledged.
        tokio::time::sleep(Duration::from_millis(500)).await;
        client
            .call_tool("emit_tools_changed", json!({}))
            .await
            .expect("emit");
        let action = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match rx.recv().await {
                    Some(Action::McpToolsRefreshed(_)) => return,
                    Some(_) => continue,
                    None => panic!("channel closed"),
                }
            }
        })
        .await;
        assert!(action.is_ok(), "tools/list_changed must reach the action channel");
    }
}
