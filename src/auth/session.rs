use std::collections::HashMap;

use crate::mcp::types::ServerIdentity;

#[derive(Debug, Clone)]
pub enum ServerCredentials {
    OAuth {
        access_token: String,
        #[allow(dead_code)]
        refresh_token: String,
        #[allow(dead_code)]
        expires_at: i64,
        #[allow(dead_code)]
        audience: String,
    },
    ApiKey {
        key: String,
    },
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub account_id: String,
    pub display_name: String,
    #[allow(dead_code)]
    pub entity_type: String,
    pub credentials: HashMap<ServerIdentity, ServerCredentials>,
}

impl AuthSession {
    /// Build HTTP headers for a specific server's MCP client transport.
    pub fn headers_for(&self, server: &ServerIdentity) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        if let Some(creds) = self.credentials.get(server) {
            match creds {
                ServerCredentials::OAuth { access_token, .. } => {
                    headers.insert(
                        "Authorization".to_string(),
                        format!("Bearer {access_token}"),
                    );
                }
                ServerCredentials::ApiKey { key } => {
                    headers.insert("X-API-Key".to_string(), key.clone());
                }
            }
        }
        headers
    }
}
