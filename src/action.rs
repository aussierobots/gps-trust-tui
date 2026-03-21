use crate::mcp::types::ServerIdentity;

#[derive(Debug, Clone)]
pub enum Action {
    Tick,
    Render,
    Quit,

    // Auth lifecycle
    AuthStart,
    AuthSuccess(crate::auth::session::AuthSession),
    AuthFailure(String),

    // MCP lifecycle
    McpConnecting(ServerIdentity),
    McpConnected(ServerIdentity),
    McpDisconnected(ServerIdentity),
    McpError(ServerIdentity, String),
    McpToolsRefreshed(ServerIdentity),
    McpProgress {
        server: ServerIdentity,
        progress_token: String,
        progress: f64,
        total: Option<f64>,
        message: Option<String>,
    },
    McpToolResult(Box<turul_mcp_protocol::CallToolResult>),
    McpTaskCreated(Box<crate::mcp::types::ActiveTask>),
    McpTaskUpdate(Box<crate::mcp::types::ActiveTask>),

    // Tool interaction
    ToolSelected(usize),
    ToolExecute,
    ToolCancel,

    // Form interaction
    FormFieldNext,
    FormFieldPrev,
    FormFieldEdit,
    FormFieldToggle,
    FormEnumNext,
    FormEnumPrev,
    FormInputChar(char),
    FormInputBackspace,
    FormSubmit,
    FormCancel,

    // UI navigation
    FocusNext,
    FocusPrev,
    ScrollUp,
    ScrollDown,
    FilterStart,
    FilterClear,
    FilterChar(char),
    FilterBackspace,
    Enter,
    Escape,

    // Result tabs
    ResultNextTab,

    // Paste
    PasteText(String),

    // Reconnect
    Reconnect,

    // Bulk data
    ToolsLoaded(Vec<crate::mcp::types::ToolEntry>),
}
