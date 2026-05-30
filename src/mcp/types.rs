use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use turul_mcp_protocol::Task;
use turul_mcp_protocol::Tool;

/// Static configuration for one MCP server.
///
/// Wrapped in [`ServerId`] (an `Arc`) so the identity handle carries its own
/// display strings and endpoint without needing a separate registry lookup at
/// every render/call site.
#[derive(Debug)]
pub struct ServerConfig {
    /// Stable key used for identity/equality/hashing (e.g. "user", "agent").
    pub key: String,
    /// Human-readable label (e.g. "User", "Agent").
    pub label: String,
    /// Short prefix badge for the tool list (e.g. "U", "A").
    pub prefix: String,
    /// MCP endpoint URL — also the OAuth audience / RFC 8707 resource indicator.
    pub url: String,
    /// Whether the entity_info identity bootstrap runs against this server.
    pub is_identity_provider: bool,
}

/// Cheap, clonable handle to a configured server. Equality and hashing are by
/// `key` only, so it works as a `HashMap` key while still carrying display data.
#[derive(Debug, Clone)]
pub struct ServerId(Arc<ServerConfig>);

impl ServerId {
    pub fn new(config: ServerConfig) -> Self {
        Self(Arc::new(config))
    }

    pub fn key(&self) -> &str {
        &self.0.key
    }

    pub fn label(&self) -> &str {
        &self.0.label
    }

    pub fn prefix(&self) -> &str {
        &self.0.prefix
    }

    pub fn url(&self) -> &str {
        &self.0.url
    }

    pub fn is_identity_provider(&self) -> bool {
        self.0.is_identity_provider
    }
}

impl PartialEq for ServerId {
    fn eq(&self, other: &Self) -> bool {
        self.0.key == other.0.key
    }
}

impl Eq for ServerId {}

impl Hash for ServerId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.key.hash(state);
    }
}

impl std::fmt::Display for ServerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.label)
    }
}

/// Ordered set of configured servers. This is the single place the server list
/// is defined; making it config-driven is a later slice (see ADR-0001).
#[derive(Debug, Clone)]
pub struct ServerRegistry {
    servers: Vec<ServerId>,
}

impl ServerRegistry {
    /// The current default fleet: User + Agent.
    pub fn user_agent(user_url: &str, agent_url: &str) -> Self {
        Self {
            servers: vec![
                ServerId::new(ServerConfig {
                    key: "user".to_string(),
                    label: "User".to_string(),
                    prefix: "U".to_string(),
                    url: user_url.to_string(),
                    is_identity_provider: true,
                }),
                ServerId::new(ServerConfig {
                    key: "agent".to_string(),
                    label: "Agent".to_string(),
                    prefix: "A".to_string(),
                    url: agent_url.to_string(),
                    is_identity_provider: false,
                }),
            ],
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, ServerId> {
        self.servers.iter()
    }

    /// The server whose `entity_info` call resolves account identity.
    pub fn identity_provider(&self) -> Option<&ServerId> {
        self.servers.iter().find(|s| s.is_identity_provider())
    }
}

/// A tool entry combining the MCP Tool definition with its server origin.
#[derive(Debug, Clone)]
pub struct ToolEntry {
    pub server: ServerId,
    pub tool: Tool,
}

impl ToolEntry {
    /// Human-readable name: annotation title if present, otherwise the tool name.
    pub fn display_name(&self) -> &str {
        self.tool
            .annotations
            .as_ref()
            .and_then(|a| a.title.as_deref())
            .unwrap_or(&self.tool.name)
    }

    /// Returns the task support badge for display: [T!], [T?], or empty.
    pub fn task_badge(&self) -> &'static str {
        match self.tool.execution.as_ref().and_then(|e| e.task_support.as_ref()) {
            Some(turul_mcp_protocol::TaskSupport::Required) => "[T!]",
            Some(turul_mcp_protocol::TaskSupport::Optional) => "[T?]",
            _ => "",
        }
    }
}

/// Request to call a tool, with arguments ready for dispatch.
#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub server: ServerId,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Tracks an in-flight task.
#[derive(Debug, Clone)]
pub struct ActiveTask {
    #[allow(dead_code)]
    pub server: ServerId,
    #[allow(dead_code)]
    pub task: Task,
    pub tool_name: String,
    pub progress: Option<f64>,
    pub total: Option<f64>,
    pub message: Option<String>,
}

/// Capabilities detected from a connected server.
#[derive(Debug, Clone, Default)]
pub struct ServerCaps {
    pub tools_list_changed: bool,
    pub tasks_tool_call: bool,
    pub tasks_cancel: bool,
    pub tasks_list: bool,
}

/// Policy for injecting managed fields (e.g. account_id) into tool call arguments.
///
/// The TUI always injects these fields so the user cannot forge them.
#[derive(Debug, Clone)]
pub struct ManagedFieldsPolicy {
    fields: HashMap<String, serde_json::Value>,
}

impl ManagedFieldsPolicy {
    pub fn new(account_id: &str) -> Self {
        let mut fields = HashMap::new();
        fields.insert(
            "account_id".to_string(),
            serde_json::Value::String(account_id.to_string()),
        );
        Self { fields }
    }

    /// Inject managed fields into tool call arguments.
    ///
    /// Returns an error if the arguments are not a JSON object.
    pub fn inject(&self, args: &mut serde_json::Value) -> Result<(), String> {
        let obj = args
            .as_object_mut()
            .ok_or("Tool arguments must be a JSON object")?;
        for (key, value) in &self.fields {
            obj.insert(key.clone(), value.clone());
        }
        Ok(())
    }
}
