use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::auth::session::{AuthSession, ServerCredentials};
use crate::auth::token_store::{StoredToken, TokenStore};
use crate::mcp::types::ServerIdentity;

const AUTH_BASE: &str = "https://auth.aussierobots.com.au";
const REDIRECT_URI: &str = "http://127.0.0.1:19876/callback";
const CALLBACK_ADDR: &str = "127.0.0.1:19876";

/// Run the full OAuth 2.1 + PKCE flow for both MCP server audiences.
pub async fn authenticate(user_url: &str, agent_url: &str) -> Result<AuthSession> {
    let http = reqwest::Client::new();
    let mut store = TokenStore::load();

    // Step 1: Dynamic Client Registration (or reuse cached client_id)
    let client_id = match &store.dcr_client_id {
        Some(id) => {
            info!("Reusing cached DCR client_id");
            id.clone()
        }
        None => {
            info!("Performing dynamic client registration");
            let id = register_client(&http).await?;
            store.dcr_client_id = Some(id.clone());
            store.save().context("failed to save DCR client_id")?;
            id
        }
    };

    // Step 2: Obtain tokens for each audience
    let audiences = [
        (ServerIdentity::User, user_url),
        (ServerIdentity::Agent, agent_url),
    ];

    let mut credentials = HashMap::new();
    let mut account_id = String::new();

    for (server, audience) in &audiences {
        // Try refresh first if we have a stored token
        if let Some(stored) = store.tokens.get(*audience) {
            match refresh_token(&http, &client_id, &stored.refresh_token).await {
                Ok(token_resp) => {
                    info!(audience = %audience, "Refreshed token successfully");
                    let expires_at = chrono_now() + token_resp.expires_in;
                    account_id = stored.account_id.clone();

                    credentials.insert(
                        *server,
                        ServerCredentials::OAuth {
                            access_token: token_resp.access_token.clone(),
                            refresh_token: token_resp
                                .refresh_token
                                .clone()
                                .unwrap_or_else(|| stored.refresh_token.clone()),
                            expires_at,
                            audience: audience.to_string(),
                        },
                    );

                    // Update stored token
                    store.tokens.insert(
                        audience.to_string(),
                        StoredToken {
                            refresh_token: token_resp
                                .refresh_token
                                .unwrap_or_else(|| stored.refresh_token.clone()),
                            expires_at,
                            account_id: account_id.clone(),
                        },
                    );
                    continue;
                }
                Err(e) => {
                    warn!(audience = %audience, error = %e, "Refresh failed, falling back to authorization");
                }
            }
        }

        // Full authorization flow
        let token_resp = authorize(&http, &client_id, audience).await?;
        let expires_at = chrono_now() + token_resp.expires_in;

        // Extract account_id from JWT sub claim
        let jwt_account_id = extract_sub_from_jwt(&token_resp.access_token)?;
        if account_id.is_empty() {
            account_id = jwt_account_id.clone();
        }

        credentials.insert(
            *server,
            ServerCredentials::OAuth {
                access_token: token_resp.access_token.clone(),
                refresh_token: token_resp
                    .refresh_token
                    .clone()
                    .unwrap_or_default(),
                expires_at,
                audience: audience.to_string(),
            },
        );

        store.tokens.insert(
            audience.to_string(),
            StoredToken {
                refresh_token: token_resp.refresh_token.unwrap_or_default(),
                expires_at,
                account_id: jwt_account_id,
            },
        );
    }

    store.save().context("failed to save tokens")?;

    // Bootstrap entity name via User MCP entity_info call
    let (display_name, entity_type) =
        resolve_entity_name(user_url, &credentials, &account_id).await;

    info!(account_id = %account_id, display_name = %display_name, "OAuth authentication successful");

    Ok(AuthSession {
        account_id,
        display_name,
        entity_type,
        credentials,
    })
}

/// Call entity_info on the User MCP server to resolve the account display name.
/// Best-effort: falls back to account_id if the call fails.
async fn resolve_entity_name(
    user_url: &str,
    credentials: &HashMap<ServerIdentity, ServerCredentials>,
    account_id: &str,
) -> (String, String) {
    use turul_mcp_client::config::{ClientConfig, ConnectionConfig};
    use turul_mcp_client::McpClientBuilder;
    use turul_mcp_protocol::ContentBlock;

    let headers: HashMap<String, String> = match credentials.get(&ServerIdentity::User) {
        Some(ServerCredentials::OAuth { access_token, .. }) => {
            let mut h = HashMap::new();
            h.insert("Authorization".to_string(), format!("Bearer {access_token}"));
            h
        }
        _ => return (account_id.to_string(), "account".to_string()),
    };

    let config = ClientConfig {
        connection: ConnectionConfig {
            headers: Some(headers),
            ..Default::default()
        },
        ..Default::default()
    };

    let Ok(builder) = McpClientBuilder::new().with_url(user_url) else {
        return (account_id.to_string(), "account".to_string());
    };
    let client = builder.with_config(config).build();

    if let Err(e) = client.connect().await {
        warn!(error = %e, "resolve_entity_name: MCP connect failed");
        return (account_id.to_string(), "account".to_string());
    }

    let result = client.call_tool("entity_info", serde_json::json!({})).await;
    let _ = client.disconnect().await;

    match result {
        Ok(call_result) => {
            let text = call_result.content.into_iter().find_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(text),
                _ => None,
            });
            if let Some(text) = text {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                    let name = parsed["entityName"]
                        .as_str()
                        .unwrap_or(account_id)
                        .to_string();
                    let etype = parsed["entityType"]
                        .as_str()
                        .unwrap_or("account")
                        .to_string();
                    return (name, etype);
                }
            }
            (account_id.to_string(), "account".to_string())
        }
        Err(e) => {
            warn!(error = %e, "resolve_entity_name: entity_info call failed");
            (account_id.to_string(), "account".to_string())
        }
    }
}

/// Dynamic Client Registration per RFC 7591.
async fn register_client(http: &reqwest::Client) -> Result<String> {
    let url = format!("{AUTH_BASE}/register");
    let body = serde_json::json!({
        "client_name": "GPS Trust MCP TUI",
        "redirect_uris": [REDIRECT_URI],
        "token_endpoint_auth_method": "none",
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"]
    });

    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("DCR request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("DCR failed with status {status}: {text}");
    }

    let data: serde_json::Value = resp.json().await.context("DCR response parse failed")?;
    data["client_id"]
        .as_str()
        .map(|s| s.to_string())
        .context("DCR response missing client_id")
}

/// Run the authorization code + PKCE flow for a single audience.
/// Resolve the scopes to request for a given audience, matching the auth
/// server's `DCR_AUDIENCE_SCOPE_POLICY`.
fn scopes_for_audience(audience: &str) -> &'static str {
    if audience.contains("agent.aussierobots.com.au")
        || audience.contains("pf.aussierobots.com.au")
    {
        "mcp:read mcp:write"
    } else {
        "mcp:read"
    }
}

async fn authorize(
    http: &reqwest::Client,
    client_id: &str,
    audience: &str,
) -> Result<TokenResponse> {
    let (verifier, challenge) = generate_pkce();
    let state = generate_state();
    let scope = urlencoded(scopes_for_audience(audience));

    // Build authorization URL
    let auth_url = format!(
        "{AUTH_BASE}/authorize?\
         response_type=code\
         &client_id={client_id}\
         &redirect_uri={redirect}\
         &code_challenge={challenge}\
         &code_challenge_method=S256\
         &scope={scope}\
         &state={state}\
         &resource={audience}",
        redirect = urlencoded(REDIRECT_URI),
        audience = urlencoded(audience),
    );

    // Start callback listener BEFORE opening the browser
    let listener = TcpListener::bind(CALLBACK_ADDR)
        .await
        .context("failed to bind callback listener on 127.0.0.1:19876")?;

    // Open browser
    info!("Opening browser for OAuth authorization");
    if let Err(e) = open::that(&auth_url) {
        warn!(error = %e, "Failed to open browser — please open this URL manually:\n{auth_url}");
    }

    // Wait for the callback
    let code = wait_for_callback(&listener, &state).await?;

    // Exchange code for tokens
    let token_resp = exchange_code(http, client_id, &code, &verifier).await?;
    Ok(token_resp)
}

/// Wait for the OAuth callback on the localhost listener.
async fn wait_for_callback(listener: &TcpListener, expected_state: &str) -> Result<String> {
    let (mut stream, _addr) = listener
        .accept()
        .await
        .context("failed to accept callback connection")?;

    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .context("failed to read callback request")?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse GET /callback?code=...&state=... HTTP/1.1
    let query = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|path| path.split('?').nth(1))
        .unwrap_or("");

    let params: HashMap<&str, &str> = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            Some((parts.next()?, parts.next()?))
        })
        .collect();

    let state = params.get("state").context("callback missing state param")?;
    if *state != expected_state {
        bail!("OAuth state mismatch");
    }

    let code = params
        .get("code")
        .context("callback missing code param")?
        .to_string();

    // Send a friendly HTML response
    let body = "\
<html><body>\
<script>window.close();</script>\
<h2>Authentication successful</h2>\
<p>This tab should close automatically. If not, you may close it.</p>\
</body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;

    Ok(code)
}

/// Exchange an authorization code for tokens.
async fn exchange_code(
    http: &reqwest::Client,
    client_id: &str,
    code: &str,
    verifier: &str,
) -> Result<TokenResponse> {
    let url = format!("{AUTH_BASE}/token");
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("redirect_uri", REDIRECT_URI),
        ("code_verifier", verifier),
    ];

    let resp = http
        .post(&url)
        .form(&params)
        .send()
        .await
        .context("token exchange request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("token exchange failed with status {status}: {text}");
    }

    resp.json::<TokenResponse>()
        .await
        .context("failed to parse token response")
}

/// Refresh an access token using a refresh token.
async fn refresh_token(
    http: &reqwest::Client,
    client_id: &str,
    refresh: &str,
) -> Result<TokenResponse> {
    let url = format!("{AUTH_BASE}/token");
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", client_id),
    ];

    let resp = http
        .post(&url)
        .form(&params)
        .send()
        .await
        .context("token refresh request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("token refresh failed with status {status}: {text}");
    }

    resp.json::<TokenResponse>()
        .await
        .context("failed to parse refresh token response")
}

/// Generate a PKCE S256 verifier + challenge pair.
fn generate_pkce() -> (String, String) {
    let verifier: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(128)
        .map(char::from)
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());
    (verifier, challenge)
}

/// Generate a random state string for CSRF protection.
fn generate_state() -> String {
    rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

/// Extract the `sub` claim from a JWT without verifying the signature.
fn extract_sub_from_jwt(token: &str) -> Result<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        bail!("invalid JWT structure");
    }
    // Decode the payload (second segment)
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .context("failed to base64-decode JWT payload")?;
    let payload: serde_json::Value =
        serde_json::from_slice(&payload_bytes).context("failed to parse JWT payload JSON")?;
    payload["sub"]
        .as_str()
        .map(|s| s.to_string())
        .context("JWT payload missing sub claim")
}

/// Minimal percent-encoding for URL query values.
fn urlencoded(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Current unix timestamp in seconds.
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}
