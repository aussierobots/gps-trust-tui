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
    /// OAuth scope to request for this audience (must match the auth server's
    /// DCR_AUDIENCE_SCOPE_POLICY).
    pub scope: String,
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

    /// Build a handle from a key + URL, filling display metadata (and the
    /// identity-provider role) from the known-server catalog, or deriving it
    /// for unknown keys.
    pub fn from_key_url(key: &str, url: &str) -> Self {
        let (label, prefix, is_identity_provider) = known_server_meta(key);
        ServerId::new(ServerConfig {
            key: key.to_string(),
            label,
            prefix,
            url: url.to_string(),
            is_identity_provider,
            scope: known_scope(key).to_string(),
        })
    }

    /// OAuth scope to request for this server's audience.
    pub fn scope(&self) -> &str {
        &self.0.scope
    }
}

/// Catalog of known servers: (key, label, prefix, default_url, is_identity_provider).
/// Single source of truth for the fleet — `--server <key>` resolves a default
/// URL here, and display metadata is looked up from it.
/// (key, label, prefix, default_url, is_identity_provider, scope). Scopes must
/// match the auth server's DCR_AUDIENCE_SCOPE_POLICY.
const KNOWN_SERVERS: &[(&str, &str, &str, &str, bool, &str)] = &[
    ("user", "User", "U", "https://gt.aussierobots.com.au/mcp", true, "mcp:read mcp:write"),
    ("agent", "Agent", "A", "https://agent.aussierobots.com.au/mcp", false, "mcp:read mcp:write"),
    ("pf", "Particle Filter", "P", "https://pf.aussierobots.com.au/mcp", false, "mcp:read mcp:write"),
    ("sv-track", "SV Track", "T", "https://st.aussierobots.com.au/mcp", false, "mcp:read"),
    ("space-data", "Space Data", "D", "https://sd.aussierobots.com.au/mcp", false, "mcp:read"),
];

/// Display metadata + identity role for known server keys; derived otherwise.
fn known_server_meta(key: &str) -> (String, String, bool) {
    if let Some((_, label, prefix, _, idp, _)) = KNOWN_SERVERS.iter().find(|(k, ..)| *k == key) {
        return (label.to_string(), prefix.to_string(), *idp);
    }
    let prefix = key
        .chars()
        .next()
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    (key.to_string(), prefix, false)
}

/// Default endpoint URL for a known server key, if any.
fn known_default_url(key: &str) -> Option<&'static str> {
    KNOWN_SERVERS
        .iter()
        .find(|(k, ..)| *k == key)
        .map(|(_, _, _, url, _, _)| *url)
}

/// OAuth scope to request for a known server key; read-only for unknown keys.
fn known_scope(key: &str) -> &'static str {
    KNOWN_SERVERS
        .iter()
        .find(|(k, ..)| *k == key)
        .map(|(_, _, _, _, _, scope)| *scope)
        .unwrap_or("mcp:read")
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
    /// Build a registry from an explicit ordered list of servers.
    pub fn new(servers: Vec<ServerId>) -> Self {
        Self { servers }
    }

    /// Build the default User + Agent fleet plus any additional servers given
    /// as `KEY=URL` specs (the `--server` flag / config). The single place
    /// "add a server" becomes a data operation rather than code.
    pub fn from_specs(user_url: &str, agent_url: &str, extra: &[String]) -> Result<Self, String> {
        let mut servers = vec![
            ServerId::from_key_url("user", user_url),
            ServerId::from_key_url("agent", agent_url),
        ];
        for spec in extra {
            let (key, url) = match spec.split_once('=') {
                Some((k, u)) => {
                    if k.is_empty() || u.is_empty() {
                        return Err(format!(
                            "--server KEY and URL must both be non-empty: '{spec}'"
                        ));
                    }
                    (k.to_string(), u.to_string())
                }
                None => {
                    // Bare key (e.g. `--server sv-track`) → known default URL.
                    let url = known_default_url(spec).ok_or_else(|| {
                        format!("--server '{spec}' has no known default URL; use KEY=URL")
                    })?;
                    (spec.clone(), url.to_string())
                }
            };
            let id = ServerId::from_key_url(&key, &url);
            // Override an existing server by key (e.g. a custom --server user=…),
            // otherwise append.
            if let Some(existing) = servers.iter_mut().find(|s| s.key() == key) {
                *existing = id;
            } else {
                servers.push(id);
            }
        }
        Ok(Self::new(servers))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(key: &str, label: &str, prefix: &str, url: &str, idp: bool) -> ServerId {
        ServerId::new(ServerConfig {
            key: key.to_string(),
            label: label.to_string(),
            prefix: prefix.to_string(),
            url: url.to_string(),
            is_identity_provider: idp,
            scope: "mcp:read".to_string(),
        })
    }

    #[test]
    fn user_agent_registry_has_user_then_agent() {
        let reg = ServerRegistry::from_specs("https://u/mcp", "https://a/mcp", &[]).unwrap();
        let ids: Vec<&ServerId> = reg.iter().collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].key(), "user");
        assert_eq!(ids[0].url(), "https://u/mcp");
        assert_eq!(ids[1].key(), "agent");
        assert_eq!(ids[1].prefix(), "A");
    }

    #[test]
    fn identity_provider_is_user() {
        let reg = ServerRegistry::from_specs("https://u/mcp", "https://a/mcp", &[]).unwrap();
        let provider = reg.identity_provider().expect("registry has an identity provider");
        assert_eq!(provider.key(), "user");
        assert!(provider.is_identity_provider());
    }

    #[test]
    fn server_id_equality_and_hashing_is_by_key_only() {
        // Two handles with the same key but different config are equal and hash
        // alike — so a ServerId built anywhere resolves the same map entry.
        let a = cfg("x", "X", "X", "url-1", false);
        let b = cfg("x", "Different", "Y", "url-2", true);
        assert_eq!(a, b);

        let mut m = HashMap::new();
        m.insert(a.clone(), 42);
        assert_eq!(m.get(&b), Some(&42));
    }

    #[test]
    fn from_specs_appends_extra_servers_with_catalog_metadata() {
        let reg = ServerRegistry::from_specs(
            "https://u/mcp",
            "https://a/mcp",
            &["sv-track=https://st/mcp".to_string()],
        )
        .unwrap();
        let ids: Vec<&ServerId> = reg.iter().collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[2].key(), "sv-track");
        assert_eq!(ids[2].label(), "SV Track");
        assert_eq!(ids[2].prefix(), "T");
        assert_eq!(ids[2].url(), "https://st/mcp");
        assert!(!ids[2].is_identity_provider());
    }

    #[test]
    fn from_specs_rejects_malformed_spec() {
        // Bare unknown key (no '=' and no catalog default) is rejected.
        assert!(ServerRegistry::from_specs("u", "a", &["noequals".to_string()]).is_err());
        assert!(ServerRegistry::from_specs("u", "a", &["=onlyurl".to_string()]).is_err());
        assert!(ServerRegistry::from_specs("u", "a", &["key=".to_string()]).is_err());
    }

    #[test]
    fn from_specs_resolves_bare_known_key_to_default_url() {
        let reg = ServerRegistry::from_specs("u", "a", &["sv-track".to_string()]).unwrap();
        let ids: Vec<&ServerId> = reg.iter().collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[2].key(), "sv-track");
        assert_eq!(ids[2].url(), "https://st.aussierobots.com.au/mcp");
        assert_eq!(ids[2].label(), "SV Track");
    }

    #[test]
    fn from_specs_overrides_existing_server_by_key() {
        let reg =
            ServerRegistry::from_specs("u", "a", &["user=https://custom/mcp".to_string()]).unwrap();
        let ids: Vec<&ServerId> = reg.iter().collect();
        assert_eq!(ids.len(), 2); // user replaced in place, not appended
        assert_eq!(ids[0].key(), "user");
        assert_eq!(ids[0].url(), "https://custom/mcp");
    }

    #[test]
    fn known_servers_request_policy_matching_scopes() {
        // gt/agent/pf read+write; sv-track/space-data read-only (auth ADR-0005).
        assert_eq!(known_scope("user"), "mcp:read mcp:write");
        assert_eq!(known_scope("agent"), "mcp:read mcp:write");
        assert_eq!(known_scope("pf"), "mcp:read mcp:write");
        assert_eq!(known_scope("sv-track"), "mcp:read");
        assert_eq!(known_scope("space-data"), "mcp:read");
        assert_eq!(known_scope("unknown"), "mcp:read"); // safe default
        assert_eq!(
            ServerId::from_key_url("user", "https://gt/mcp").scope(),
            "mcp:read mcp:write"
        );
    }

    #[test]
    fn managed_fields_injects_account_id_preserving_other_args() {
        let policy = ManagedFieldsPolicy::new("A#123");
        let mut args = serde_json::json!({ "device_id": "D#x" });
        policy.inject(&mut args).unwrap();
        assert_eq!(args["account_id"], "A#123");
        assert_eq!(args["device_id"], "D#x");
    }

    #[test]
    fn managed_fields_rejects_non_object_args() {
        let policy = ManagedFieldsPolicy::new("A#123");
        let mut args = serde_json::json!("not an object");
        assert!(policy.inject(&mut args).is_err());
    }
}
