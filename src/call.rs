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

/// Format a JSON value in the requested output format.
pub fn format_output(value: &serde_json::Value, format: &str) -> Result<String> {
    match format {
        "json" => serde_json::to_string_pretty(value).context("JSON serialization failed"),
        "yaml" => serde_yml::to_string(value).context("YAML serialization failed"),
        "toml" => {
            // TOML requires a top-level table. Wrap non-table values.
            let toml_value = json_to_toml(value);
            toml::to_string_pretty(&toml_value).context("TOML serialization failed")
        }
        "toon" => {
            toon_format::encode::encode_default(value)
                .map_err(|e| anyhow::anyhow!("TOON encoding failed: {e}"))
        }
        _ => bail!("unsupported output format: {format}"),
    }
}

/// Convert a serde_json::Value to a toml::Value.
/// TOML doesn't support null or top-level non-tables, so we handle those.
fn json_to_toml(value: &serde_json::Value) -> toml::Value {
    match value {
        serde_json::Value::Null => toml::Value::String("null".to_string()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            toml::Value::Array(arr.iter().map(json_to_toml).collect())
        }
        serde_json::Value::Object(map) => {
            let mut table = toml::map::Map::new();
            for (k, v) in map {
                table.insert(k.clone(), json_to_toml(v));
            }
            toml::Value::Table(table)
        }
    }
}

use crate::mcp::types::ToolEntry;

// ---------------------------------------------------------------------------
// Shared connect helper
// ---------------------------------------------------------------------------

/// Auth + connect + bootstrap + list tools. Used by all CLI subcommands.
async fn connect_and_list(
    api_key: Option<String>,
    oauth: bool,
    user_url: &str,
    agent_url: &str,
) -> Result<(McpManager, Vec<ToolEntry>)> {
    let auth_manager = AuthManager::new(api_key, oauth, user_url.to_string(), agent_url.to_string());
    let session = auth_manager
        .authenticate()
        .await
        .context("authentication failed")?;

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

    let tools = mcp_manager
        .list_all_tools()
        .await
        .context("failed to list tools")?;

    Ok((mcp_manager, tools))
}

/// Find a tool by name, or error with available tool names.
fn find_tool<'a>(tools: &'a [ToolEntry], tool_name: &str) -> Result<&'a ToolEntry> {
    tools
        .iter()
        .find(|t| t.tool.name == tool_name)
        .with_context(|| {
            let available: Vec<&str> = tools.iter().map(|t| t.tool.name.as_str()).collect();
            format!(
                "tool '{}' not found. Available tools: {}",
                tool_name,
                available.join(", ")
            )
        })
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// Call a tool and print the result.
pub async fn run_call(
    tool_name: &str,
    params: &[String],
    output_format: &str,
    api_key: Option<String>,
    oauth: bool,
    user_url: &str,
    agent_url: &str,
) -> Result<()> {
    let arguments = parse_params(params)?;
    let (mcp_manager, tools) = connect_and_list(api_key, oauth, user_url, agent_url).await?;
    let tool_entry = find_tool(&tools, tool_name)?;

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

    let output = extract_json(&result);
    let formatted = format_output(&output, output_format)?;
    println!("{formatted}");

    let _ = mcp_manager.disconnect_all().await;
    Ok(())
}

/// Describe a tool — name, description, parameters, annotations.
pub async fn run_describe(
    tool_name: &str,
    output_format: &str,
    api_key: Option<String>,
    oauth: bool,
    user_url: &str,
    agent_url: &str,
) -> Result<()> {
    let (mcp_manager, tools) = connect_and_list(api_key, oauth, user_url, agent_url).await?;
    let entry = find_tool(&tools, tool_name)?;

    let output = tool_to_description(entry);
    let formatted = format_output(&output, output_format)?;
    println!("{formatted}");

    let _ = mcp_manager.disconnect_all().await;
    Ok(())
}

/// List all available tools with name, server, and title.
pub async fn run_tools(
    output_format: &str,
    api_key: Option<String>,
    oauth: bool,
    user_url: &str,
    agent_url: &str,
) -> Result<()> {
    let (mcp_manager, tools) = connect_and_list(api_key, oauth, user_url, agent_url).await?;

    let list: Vec<serde_json::Value> = tools
        .iter()
        .map(|entry| {
            serde_json::json!({
                "name": entry.tool.name,
                "server": entry.server.label(),
                "title": entry.display_name(),
                "description": entry.tool.description.as_deref().unwrap_or(""),
            })
        })
        .collect();

    let output = serde_json::Value::Array(list);
    let formatted = format_output(&output, output_format)?;
    println!("{formatted}");

    let _ = mcp_manager.disconnect_all().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tool description builder
// ---------------------------------------------------------------------------

/// Build a JSON description of a tool including parameters and annotations.
fn tool_to_description(entry: &ToolEntry) -> serde_json::Value {
    let tool = &entry.tool;

    let mut desc = serde_json::json!({
        "name": tool.name,
        "server": entry.server.label(),
        "title": entry.display_name(),
        "description": tool.description.as_deref().unwrap_or(""),
    });

    // Parameters from input_schema
    if let Some(ref props) = tool.input_schema.properties {
        let required: Vec<&str> = tool
            .input_schema
            .required
            .as_ref()
            .map(|r| r.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let params: serde_json::Map<String, serde_json::Value> = props
            .iter()
            .map(|(name, schema)| {
                let mut param = serde_json::json!({
                    "type": schema_type_str(schema),
                    "required": required.contains(&name.as_str()),
                });
                if let Some(desc) = schema_desc(schema) {
                    param["description"] = serde_json::Value::String(desc.to_string());
                }
                (name.clone(), param)
            })
            .collect();

        desc["parameters"] = serde_json::Value::Object(params);
    }

    // Annotations
    if let Some(ref ann) = tool.annotations {
        let mut annotations = serde_json::Map::new();
        if let Some(read_only) = ann.read_only_hint {
            annotations.insert("readOnly".to_string(), serde_json::Value::Bool(read_only));
        }
        if let Some(destructive) = ann.destructive_hint {
            annotations.insert(
                "destructive".to_string(),
                serde_json::Value::Bool(destructive),
            );
        }
        if let Some(idempotent) = ann.idempotent_hint {
            annotations.insert(
                "idempotent".to_string(),
                serde_json::Value::Bool(idempotent),
            );
        }
        if let Some(open_world) = ann.open_world_hint {
            annotations.insert(
                "openWorld".to_string(),
                serde_json::Value::Bool(open_world),
            );
        }
        if !annotations.is_empty() {
            desc["annotations"] = serde_json::Value::Object(annotations);
        }
    }

    // Task support
    if let Some(ref exec) = tool.execution {
        if let Some(ref ts) = exec.task_support {
            desc["taskSupport"] = serde_json::Value::String(format!("{:?}", ts).to_lowercase());
        }
    }

    desc
}

/// Extract type name from JsonSchema.
fn schema_type_str(schema: &turul_mcp_protocol::JsonSchema) -> &'static str {
    use turul_mcp_protocol::JsonSchema;
    match schema {
        JsonSchema::String { .. } => "string",
        JsonSchema::Integer { .. } => "integer",
        JsonSchema::Number { .. } => "number",
        JsonSchema::Boolean { .. } => "boolean",
        JsonSchema::Array { .. } => "array",
        JsonSchema::Object { .. } => "object",
    }
}

/// Extract description from JsonSchema.
fn schema_desc(schema: &turul_mcp_protocol::JsonSchema) -> Option<&str> {
    use turul_mcp_protocol::JsonSchema;
    match schema {
        JsonSchema::String { description, .. }
        | JsonSchema::Integer { description, .. }
        | JsonSchema::Number { description, .. }
        | JsonSchema::Boolean { description, .. }
        | JsonSchema::Array { description, .. }
        | JsonSchema::Object { description, .. } => description.as_deref(),
    }
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

    // --- format_output tests ---

    #[test]
    fn test_format_json() {
        let val = serde_json::json!({"name": "test", "count": 3});
        let out = format_output(&val, "json").unwrap();
        assert!(out.contains("\"name\": \"test\""));
        assert!(out.contains("\"count\": 3"));
    }

    #[test]
    fn test_format_yaml() {
        let val = serde_json::json!({"name": "test", "count": 3});
        let out = format_output(&val, "yaml").unwrap();
        assert!(out.contains("name: test"));
        assert!(out.contains("count: 3"));
    }

    #[test]
    fn test_format_toml() {
        let val = serde_json::json!({"name": "test", "count": 3});
        let out = format_output(&val, "toml").unwrap();
        assert!(out.contains("name = \"test\""));
        assert!(out.contains("count = 3"));
    }

    #[test]
    fn test_format_toon() {
        let val = serde_json::json!({"name": "test", "items": [1, 2, 3]});
        let out = format_output(&val, "toon").unwrap();
        // TOON format should be more compact than JSON
        assert!(out.len() < serde_json::to_string_pretty(&val).unwrap().len());
        assert!(!out.is_empty());
    }

    #[test]
    fn test_format_unsupported() {
        let val = serde_json::json!({"x": 1});
        let result = format_output(&val, "xml");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported"));
    }

    #[test]
    fn test_format_toml_with_nested() {
        let val = serde_json::json!({
            "device": {
                "id": "D#001",
                "type": "F9P"
            }
        });
        let out = format_output(&val, "toml").unwrap();
        assert!(out.contains("[device]"));
        assert!(out.contains("id = \"D#001\""));
    }
}
