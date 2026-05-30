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
use crate::mcp::types::ServerRegistry;

pub const AUTH_BASE: &str = "https://auth.aussierobots.com.au";
const REDIRECT_URI: &str = "http://127.0.0.1:19876/callback";
const CALLBACK_ADDR: &str = "127.0.0.1:19876";

/// Run the full OAuth 2.1 + PKCE flow for every configured MCP server audience.
pub async fn authenticate(registry: &ServerRegistry) -> Result<AuthSession> {
    let http = reqwest::Client::new();
    let mut store = TokenStore::load();

    // Step 1: reuse a cached DCR client_id only if it is still known to the
    // auth server. DCR clients are ephemeral (a redeploy drops them), so a
    // blindly-reused stale id fails with `invalid_client` in the browser and
    // leaves the callback listener hanging. A cheap preflight catches that and
    // re-registers instead. Any registered audience works for the probe.
    let probe_resource = registry.iter().next().map(|s| s.url().to_string());
    let cached = store.dcr_client_id.clone();
    let reuse = match (&cached, &probe_resource) {
        (Some(id), Some(resource)) => client_id_still_valid(id, resource).await,
        _ => false,
    };
    let client_id = if reuse {
        info!("Reusing cached DCR client_id");
        cached.expect("reuse implies a cached client_id")
    } else {
        if cached.is_some() {
            warn!("Cached DCR client_id is no longer valid — re-registering");
        } else {
            info!("Performing dynamic client registration");
        }
        let id = register_client(&http).await?;
        store.dcr_client_id = Some(id.clone());
        store.save().context("failed to save DCR client_id")?;
        id
    };

    // Step 2: Obtain a token for each configured server's audience
    let mut credentials = HashMap::new();
    let mut account_id = String::new();

    for server in registry.iter() {
        let audience = server.url();
        // Try refresh first if we have a stored token
        if let Some(stored) = store.tokens.get(audience) {
            match refresh_token(&http, &client_id, &stored.refresh_token).await {
                Ok(token_resp) => {
                    info!(audience = %audience, "Refreshed token successfully");
                    let expires_at = chrono_now() + token_resp.expires_in;
                    account_id = stored.account_id.clone();

                    credentials.insert(
                        server.clone(),
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

        // Full authorization flow. Fail-soft: if the auth server declines this
        // audience for the entity (e.g. invalid_request / access_denied), skip
        // the server rather than aborting login for all servers. With no
        // credential, McpManager surfaces it as Unauthorized.
        let token_resp = match authorize(&http, &client_id, audience, server.scope()).await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(audience = %audience, error = %e, "authorization declined — skipping server");
                continue;
            }
        };
        let expires_at = chrono_now() + token_resp.expires_in;

        // Extract account_id from JWT sub claim
        let jwt_account_id = extract_sub_from_jwt(&token_resp.access_token)?;
        if account_id.is_empty() {
            account_id = jwt_account_id.clone();
        }

        credentials.insert(
            server.clone(),
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

    info!(account_id = %account_id, "OAuth authentication successful");

    // Identity (display_name, entity_type) resolved later via
    // McpManager::bootstrap_identity() on the already-connected session.
    Ok(AuthSession {
        account_id,
        display_name: String::new(),
        entity_type: String::new(),
        credentials,
    })
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

/// Cheap preflight: is this cached DCR `client_id` still known to the auth
/// server? A known client redirects (3xx) to the login page; an unknown one
/// returns an `invalid_client` body. Returns false on `invalid_client` or any
/// error — re-registering is far cheaper than hanging at the browser step.
async fn client_id_still_valid(client_id: &str, resource: &str) -> bool {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let url = format!(
        "{AUTH_BASE}/authorize?response_type=code\
         &client_id={client_id}\
         &redirect_uri={redirect}\
         &code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM\
         &code_challenge_method=S256\
         &scope=mcp%3Aread\
         &state=preflight\
         &resource={resource}",
        client_id = urlencoded(client_id),
        redirect = urlencoded(REDIRECT_URI),
        resource = urlencoded(resource),
    );

    match client.get(&url).send().await {
        Ok(resp) => {
            let is_redirect = resp.status().is_redirection();
            let body = resp.text().await.unwrap_or_default();
            preflight_indicates_valid(is_redirect, &body)
        }
        Err(e) => {
            warn!(error = %e, "DCR client_id preflight failed — re-registering");
            false
        }
    }
}

/// Interpret a preflight `/authorize` response: a known client redirects to
/// login; an unknown one returns an `invalid_client` error body.
fn preflight_indicates_valid(is_redirect: bool, body: &str) -> bool {
    is_redirect || !body.contains("invalid_client")
}

/// Run the authorization code + PKCE flow for a single audience. The scope is
/// supplied by the caller from the server's `ServerConfig` (the single source
/// of truth, kept in lockstep with the auth server's `DCR_AUDIENCE_SCOPE_POLICY`).
async fn authorize(
    http: &reqwest::Client,
    client_id: &str,
    audience: &str,
    scope: &str,
) -> Result<TokenResponse> {
    let (verifier, challenge) = generate_pkce();
    let state = generate_state();
    let scope = urlencoded(scope);

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
    // Loop: accept connections until we get the right callback.
    // Stale browser tabs, favicon requests, etc. are dismissed gracefully.
    loop {
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

        let state = match params.get("state") {
            Some(s) => *s,
            None => {
                // Not an OAuth callback (favicon, stale tab, etc.) — dismiss
                let _ = send_html(&mut stream, "Not an OAuth callback. You may close this tab.").await;
                continue;
            }
        };

        if state != expected_state {
            // Stale callback from a previous session — dismiss and keep waiting
            let _ = send_html(&mut stream, "Stale session. Waiting for current login...").await;
            continue;
        }

        let code = match params.get("code") {
            Some(c) => c.to_string(),
            None => {
                // Error callback (e.g. user denied consent)
                let error = params.get("error").unwrap_or(&"unknown");
                let desc = params.get("error_description").unwrap_or(&"");
                let _ = send_html(&mut stream, &format!("Authorization failed: {} {}", error, desc)).await;
                bail!("OAuth authorization denied: {} {}", error, desc);
            }
        };

        let _ = send_html(&mut stream, "\
<script>window.close();</script>\
<h2>Authentication successful</h2>\
<p>This tab should close automatically. If not, you may close it.</p>").await;

        return Ok(code);
    }
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

/// Send an HTML response on the callback stream.
async fn send_html(stream: &mut tokio::net::TcpStream, body_content: &str) -> Result<()> {
    let body = format!("<html><body>{body_content}</body></html>");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_redirect_means_valid() {
        // A known client_id redirects (303) to /login.
        assert!(preflight_indicates_valid(true, ""));
    }

    #[test]
    fn preflight_invalid_client_body_means_stale() {
        assert!(!preflight_indicates_valid(
            false,
            r#"{"error":"invalid_client","error_description":"unknown client_id"}"#
        ));
    }

    #[test]
    fn preflight_other_non_redirect_response_is_treated_valid() {
        // Some other 4xx (e.g. resource/scope quibble) is not a dead client —
        // proceed and let the real flow surface it.
        assert!(preflight_indicates_valid(false, "<html>login</html>"));
    }
}
