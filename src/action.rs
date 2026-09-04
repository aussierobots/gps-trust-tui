use crate::mcp::types::ServerId;

#[derive(Debug, Clone)]
pub enum Action {
    Quit,

    // Auth lifecycle
    AuthSuccess(crate::auth::session::AuthSession),

    // MCP lifecycle
    McpConnecting(ServerId),
    McpConnected(ServerId),
    #[allow(dead_code)]
    McpError(ServerId, String),
    McpUnauthorized(ServerId),
    McpToolsRefreshed(ServerId),
    McpProgress {
        #[allow(dead_code)]
        server: ServerId,
        #[allow(dead_code)]
        progress_token: String,
        progress: f64,
        total: Option<f64>,
        message: Option<String>,
    },
    McpToolResult(Box<turul_mcp_client::CallToolResult>),
    /// Result of a background id-cache auto-fetch (see `IdFetchRequest`).
    /// Only scans the result into `id_cache` and refreshes any open form's
    /// empty carry-forward fields — never touches focus or `result_state`.
    IdCacheFetched(Box<turul_mcp_client::CallToolResult>),

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
    /// Cycle the selected form field's value backward/forward through its
    /// enum values or cached `*_id` suggestions.
    CyclePrev,
    CycleNext,
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
