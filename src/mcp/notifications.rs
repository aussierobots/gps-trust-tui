use std::collections::HashMap;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, warn};
use turul_mcp_protocol::ProgressNotificationParams;

use crate::action::Action;
use crate::mcp::types::ServerIdentity;

/// Parse a notification JSON value and dispatch the appropriate Action.
pub fn dispatch_notification(
    server: ServerIdentity,
    value: &Value,
    tx: &UnboundedSender<Action>,
) {
    let method = match value.get("method").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => {
            debug!(?value, "notification missing method field");
            return;
        }
    };

    match method {
        "notifications/tools/list_changed" => {
            debug!(server = %server, "tools list changed notification");
            let _ = tx.send(Action::McpToolsRefreshed(server));
        }
        "notifications/progress" => {
            if let Some(params_val) = value.get("params") {
                match serde_json::from_value::<ProgressNotificationParams>(params_val.clone()) {
                    Ok(params) => {
                        let token = match &params.progress_token {
                            turul_mcp_protocol::ProgressTokenValue::String(s) => s.clone(),
                            turul_mcp_protocol::ProgressTokenValue::Number(n) => n.to_string(),
                        };
                        let _ = tx.send(Action::McpProgress {
                            server,
                            progress_token: token,
                            progress: params.progress,
                            total: params.total,
                            message: params.message,
                        });
                    }
                    Err(e) => {
                        warn!(server = %server, error = %e, "failed to parse progress params");
                    }
                }
            }
        }
        other => {
            debug!(server = %server, method = other, "unhandled notification");
        }
    }
}

/// Coalesces progress notifications, keeping only the latest per (server, token).
///
/// Call `push()` for each progress action, then `drain()` on each tick to get
/// the de-duplicated batch.
#[allow(dead_code)]
pub struct NotificationCoalescer {
    buffer: HashMap<(ServerIdentity, String), Action>,
}

#[allow(dead_code)]
impl NotificationCoalescer {
    pub fn new() -> Self {
        Self {
            buffer: HashMap::new(),
        }
    }

    /// Buffer a progress action. Non-progress actions are stored with an empty token key.
    pub fn push(&mut self, action: Action) {
        match &action {
            Action::McpProgress {
                server,
                progress_token,
                ..
            } => {
                self.buffer
                    .insert((*server, progress_token.clone()), action);
            }
            _ => {
                // Non-progress actions pass through immediately via a unique key
                let key = (ServerIdentity::User, format!("__other_{}", self.buffer.len()));
                self.buffer.insert(key, action);
            }
        }
    }

    /// Drain all buffered actions, returning the latest per token.
    pub fn drain(&mut self) -> Vec<Action> {
        self.buffer.drain().map(|(_, v)| v).collect()
    }
}

impl Default for NotificationCoalescer {
    fn default() -> Self {
        Self::new()
    }
}
