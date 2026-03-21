use crate::mcp::types::ServerIdentity;

#[derive(Debug, Clone)]
pub enum Action {
    Quit,

    // Auth lifecycle
    AuthSuccess(crate::auth::session::AuthSession),

    // MCP lifecycle
    McpConnecting(ServerIdentity),
    McpConnected(ServerIdentity),
    McpDisconnected(ServerIdentity),
    McpError(ServerIdentity, String),
    McpToolsRefreshed(ServerIdentity),
    McpProgress {
        #[allow(dead_code)]
        server: ServerIdentity,
        #[allow(dead_code)]
        progress_token: String,
        progress: f64,
        total: Option<f64>,
        message: Option<String>,
    },
    McpToolResult(Box<turul_mcp_protocol::CallToolResult>),

    // Tool interaction
    #[allow(dead_code)]
    ToolCancel,

    // Form interaction
    #[allow(dead_code)]
    FormFieldToggle,
    #[allow(dead_code)]
    FormInputChar(char),
    #[allow(dead_code)]
    FormInputBackspace,

    // UI navigation
    FocusNext,
    FocusPrev,
    ScrollUp,
    ScrollDown,
    #[allow(dead_code)]
    FilterStart,
    FilterChar(char),
    FilterBackspace,
    Enter,
    Escape,

    // Result tabs
    #[allow(dead_code)]
    ResultNextTab,

    // Paste
    PasteText(String),

    // Reconnect
    #[allow(dead_code)]
    Reconnect,

    // Bulk data
    ToolsLoaded(Vec<crate::mcp::types::ToolEntry>),
}
