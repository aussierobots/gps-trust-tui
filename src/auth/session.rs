use std::collections::HashMap;

use tracing::warn;

use crate::mcp::types::ServerId;

#[derive(Debug, Clone)]
pub enum ServerCredentials {
    OAuth {
        access_token: String,
        #[allow(dead_code)]
        refresh_token: String,
        #[allow(dead_code)]
        expires_at: i64,
        /// Audience the token was minted for (RFC 8707 resource). Asserted
        /// against the target server before the token is attached.
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
    pub credentials: HashMap<ServerId, ServerCredentials>,
}

impl AuthSession {
    /// Whether we hold a credential that is actually *usable* for this server.
    /// For OAuth the token's audience must match the server (defense-in-depth
    /// against a token being routed to the wrong server); API keys carry no
    /// audience and are validated server-side, so they always qualify.
    pub fn has_usable_credential(&self, server: &ServerId) -> bool {
        match self.credentials.get(server) {
            Some(ServerCredentials::OAuth { audience, .. }) => audience == server.url(),
            Some(ServerCredentials::ApiKey { .. }) => true,
            None => false,
        }
    }

    /// Build HTTP headers for a specific server's MCP client transport.
    pub fn headers_for(&self, server: &ServerId) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        if let Some(creds) = self.credentials.get(server) {
            match creds {
                ServerCredentials::OAuth {
                    access_token,
                    audience,
                    ..
                } => {
                    // Fail closed: never attach a token minted for a different
                    // audience to this server.
                    if audience == server.url() {
                        headers.insert(
                            "Authorization".to_string(),
                            format!("Bearer {access_token}"),
                        );
                    } else {
                        warn!(
                            server = %server,
                            token_audience = %audience,
                            "credential audience mismatch — withholding token"
                        );
                    }
                }
                ServerCredentials::ApiKey { key } => {
                    headers.insert("X-API-Key".to_string(), key.clone());
                }
            }
        }
        headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st_server() -> ServerId {
        ServerId::from_key_url("sv-track", "https://st/mcp")
    }

    fn session_with(server: &ServerId, creds: ServerCredentials) -> AuthSession {
        let mut credentials = HashMap::new();
        credentials.insert(server.clone(), creds);
        AuthSession {
            account_id: String::new(),
            display_name: String::new(),
            entity_type: String::new(),
            credentials,
        }
    }

    #[test]
    fn mismatched_audience_is_unusable_and_token_withheld() {
        let st = st_server();
        let session = session_with(
            &st,
            ServerCredentials::OAuth {
                access_token: "tok".into(),
                refresh_token: String::new(),
                expires_at: 0,
                audience: "https://WRONG/mcp".into(),
            },
        );
        assert!(session.credentials.contains_key(&st)); // entry exists
        assert!(!session.has_usable_credential(&st)); // but audience mismatches
        assert!(session.headers_for(&st).is_empty()); // so no token attached
    }

    #[test]
    fn matching_audience_is_usable_and_attaches_bearer() {
        let st = st_server();
        let session = session_with(
            &st,
            ServerCredentials::OAuth {
                access_token: "tok".into(),
                refresh_token: String::new(),
                expires_at: 0,
                audience: "https://st/mcp".into(),
            },
        );
        assert!(session.has_usable_credential(&st));
        assert_eq!(
            session.headers_for(&st).get("Authorization").unwrap(),
            "Bearer tok"
        );
    }

    #[test]
    fn api_key_is_always_usable() {
        let st = st_server();
        let session = session_with(&st, ServerCredentials::ApiKey { key: "k".into() });
        assert!(session.has_usable_credential(&st));
        assert_eq!(session.headers_for(&st).get("X-API-Key").unwrap(), "k");
    }
}
