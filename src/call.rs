use anyhow::{Context, Result, bail};
use tokio::sync::mpsc;
use tracing::info;
use turul_mcp_protocol::ContentBlock;

use crate::action::Action;
use crate::auth::AuthManager;
use crate::mcp::McpManager;
use crate::mcp::types::ToolCallRequest;

/// Parse `key=value` pairs from --param flags into a JSON object.
pub fn parse_params(params: &[String]) -> Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for param in params {
        let (key, value) = param
            .split_once('=')
            .with_context(|| format!("invalid param '{}': expected key=value", param))?;

        // Try to parse value as JSON (number, bool, null, object, array).
        // Fall back to string if it doesn't parse.
        let json_value = serde_json::from_str(value).unwrap_or_else(|_| {
            serde_json::Value::String(value.to_string())
        });

        map.insert(key.to_string(), json_value);
    }
    Ok(serde_json::Value::Object(map))
}

/// Extract JSON output from a CallToolResult.
///
/// Priority: structured_content > first text block parsed as JSON > raw text as string.
pub fn extract_json(result: &turul_mcp_protocol::CallToolResult) -> serde_json::Value {
    // Prefer structured_content
    if let Some(ref structured) = result.structured_content {
        return structured.clone();
    }

    // Try first text content block
    for block in &result.content {
        if let ContentBlock::Text { text, .. } = block {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                return json;
            }
            // Not JSON — return as string value
            return serde_json::Value::String(text.clone());
        }
    }

    serde_json::Value::Null
}

/// Run a tool call from the CLI, printing JSON to stdout.
pub async fn run_call(
    tool_name: &str,
    params: &[String],
    api_key: Option<String>,
    oauth: bool,
    user_url: &str,
    agent_url: &str,
) -> Result<()> {
    let arguments = parse_params(params)?;

    // Build credentials (no MCP connection yet)
    let auth_manager = AuthManager::new(api_key, oauth, user_url.to_string(), agent_url.to_string());
    let session = auth_manager
        .authenticate()
        .await
        .context("authentication failed")?;

    // Connect MCP servers + bootstrap identity on the connected session
    let (action_tx, _action_rx) = mpsc::unbounded_channel::<Action>();
    let mut mcp_manager = McpManager::new(&session, user_url, agent_url)
        .context("failed to create MCP manager")?;
    mcp_manager
        .connect_all(action_tx)
        .await
        .context("failed to connect MCP servers")?;
    mcp_manager
        .bootstrap_identity()
        .await
        .context("failed to bootstrap identity")?;

    // Resolve which server owns the tool
    let tools = mcp_manager
        .list_all_tools()
        .await
        .context("failed to list tools")?;

    let tool_entry = tools
        .iter()
        .find(|t| t.tool.name == tool_name)
        .with_context(|| {
            let available: Vec<&str> = tools.iter().map(|t| t.tool.name.as_str()).collect();
            format!(
                "tool '{}' not found. Available tools: {}",
                tool_name,
                available.join(", ")
            )
        })?;

    info!(tool = %tool_name, server = %tool_entry.server, "Calling tool");

    let request = ToolCallRequest {
        server: tool_entry.server,
        tool_name: tool_name.to_string(),
        arguments,
    };

    let result = mcp_manager
        .call_tool(request)
        .await
        .context("tool call failed")?;

    // Check for MCP-level error
    if result.is_error == Some(true) {
        let error_text = result
            .content
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "unknown error".to_string());
        bail!("tool returned error: {}", error_text);
    }

    // Extract and print JSON
    let output = extract_json(&result);
    let json_str = serde_json::to_string_pretty(&output)?;
    println!("{json_str}");

    // Cleanup
    let _ = mcp_manager.disconnect_all().await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use turul_mcp_protocol::{CallToolResult, ContentBlock};

    #[test]
    fn test_parse_params_empty() {
        let result = parse_params(&[]).unwrap();
        assert_eq!(result, serde_json::json!({}));
    }

    #[test]
    fn test_parse_params_string_value() {
        let params = vec!["robot_id=R#abc123".to_string()];
        let result = parse_params(&params).unwrap();
        assert_eq!(result, serde_json::json!({"robot_id": "R#abc123"}));
    }

    #[test]
    fn test_parse_params_multiple() {
        let params = vec![
            "device_id=D#001".to_string(),
            "limit=50".to_string(),
            "include_history=true".to_string(),
        ];
        let result = parse_params(&params).unwrap();
        assert_eq!(
            result,
            serde_json::json!({
                "device_id": "D#001",
                "limit": 50,
                "include_history": true
            })
        );
    }

    #[test]
    fn test_parse_params_numeric_string() {
        // Pure numbers should parse as numbers
        let params = vec!["precision=9".to_string()];
        let result = parse_params(&params).unwrap();
        assert_eq!(result["precision"], serde_json::json!(9));
    }

    #[test]
    fn test_parse_params_float() {
        let params = vec!["latitude=-33.8688".to_string()];
        let result = parse_params(&params).unwrap();
        assert_eq!(result["latitude"], serde_json::json!(-33.8688));
    }

    #[test]
    fn test_parse_params_boolean() {
        let params = vec!["enabled=true".to_string(), "verbose=false".to_string()];
        let result = parse_params(&params).unwrap();
        assert_eq!(result["enabled"], serde_json::json!(true));
        assert_eq!(result["verbose"], serde_json::json!(false));
    }

    #[test]
    fn test_parse_params_json_object() {
        let params = vec![r#"config={"key":"val"}"#.to_string()];
        let result = parse_params(&params).unwrap();
        assert_eq!(result["config"], serde_json::json!({"key": "val"}));
    }

    #[test]
    fn test_parse_params_value_with_equals() {
        // Only split on first '='
        let params = vec!["filter=a=b".to_string()];
        let result = parse_params(&params).unwrap();
        assert_eq!(result["filter"], serde_json::json!("a=b"));
    }

    #[test]
    fn test_parse_params_invalid_no_equals() {
        let params = vec!["no_equals_here".to_string()];
        let result = parse_params(&params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("expected key=value"));
    }

    #[test]
    fn test_extract_json_structured_content() {
        let result = CallToolResult {
            content: vec![],
            is_error: None,
            structured_content: Some(serde_json::json!({"count": 5})),
            meta: None,
        };
        assert_eq!(extract_json(&result), serde_json::json!({"count": 5}));
    }

    #[test]
    fn test_extract_json_text_json() {
        let result = CallToolResult {
            content: vec![ContentBlock::Text {
                text: r#"{"devices": []}"#.to_string(),
                annotations: None,
                meta: None,
            }],
            is_error: None,
            structured_content: None,
            meta: None,
        };
        assert_eq!(extract_json(&result), serde_json::json!({"devices": []}));
    }

    #[test]
    fn test_extract_json_text_plain() {
        let result = CallToolResult {
            content: vec![ContentBlock::Text {
                text: "Hello world".to_string(),
                annotations: None,
                meta: None,
            }],
            is_error: None,
            structured_content: None,
            meta: None,
        };
        assert_eq!(extract_json(&result), serde_json::json!("Hello world"));
    }

    #[test]
    fn test_extract_json_empty() {
        let result = CallToolResult {
            content: vec![],
            is_error: None,
            structured_content: None,
            meta: None,
        };
        assert_eq!(extract_json(&result), serde_json::Value::Null);
    }

    #[test]
    fn test_extract_json_structured_takes_priority() {
        let result = CallToolResult {
            content: vec![ContentBlock::Text {
                text: r#"{"from": "text"}"#.to_string(),
                annotations: None,
                meta: None,
            }],
            is_error: None,
            structured_content: Some(serde_json::json!({"from": "structured"})),
            meta: None,
        };
        assert_eq!(
            extract_json(&result),
            serde_json::json!({"from": "structured"})
        );
    }

    #[test]
    fn test_extract_json_skips_non_text_blocks() {
        let result = CallToolResult {
            content: vec![
                ContentBlock::Image {
                    data: "abc".to_string(),
                    mime_type: "image/png".to_string(),
                    annotations: None,
                    meta: None,
                },
                ContentBlock::Text {
                    text: r#"{"found": true}"#.to_string(),
                    annotations: None,
                    meta: None,
                },
            ],
            is_error: None,
            structured_content: None,
            meta: None,
        };
        assert_eq!(extract_json(&result), serde_json::json!({"found": true}));
    }
}
