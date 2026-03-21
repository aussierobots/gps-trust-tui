use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use turul_mcp_protocol::Task;
use turul_mcp_protocol::Tool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ServerIdentity {
    User,
    Agent,
}

impl ServerIdentity {
    pub fn label(&self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Agent => "Agent",
        }
    }

    pub fn prefix(&self) -> &'static str {
        match self {
            Self::User => "U",
            Self::Agent => "A",
        }
    }
}

impl std::fmt::Display for ServerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A tool entry combining the MCP Tool definition with its server origin.
#[derive(Debug, Clone)]
pub struct ToolEntry {
    pub server: ServerIdentity,
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
    pub server: ServerIdentity,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Tracks an in-flight task.
#[derive(Debug, Clone)]
pub struct ActiveTask {
    pub server: ServerIdentity,
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
