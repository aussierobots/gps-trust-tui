mod action;
mod app;
mod auth;
mod call;
mod event;
mod mcp;
mod tui;
mod ui;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};

use crate::action::Action;
use crate::app::App;
use crate::auth::AuthManager;
use crate::event::EventHandler;
use crate::mcp::McpManager;
use crate::mcp::types::ToolCallRequest;
use crate::ui::result_view::ResultState;
use crate::ui::tool_form::assemble_args;

#[derive(Parser, Debug)]
#[command(name = "gttui", about = "GPS Trust MCP Terminal UI")]
struct Cli {
    /// API key for MCP server authentication
    #[arg(long, env = "GPS_TRUST_API_KEY")]
    api_key: Option<String>,

    /// Use OAuth 2.1 authentication [default: true, --no-oauth to disable]
    #[arg(long, default_value_t = true, overrides_with = "no_oauth")]
    oauth: bool,

    /// Disable OAuth (use API key only)
    #[arg(long = "no-oauth", action = clap::ArgAction::SetTrue)]
    no_oauth: bool,

    /// User MCP server URL
    #[arg(long, default_value = "https://gt.aussierobots.com.au/mcp")]
    user_url: String,

    /// Agent MCP server URL
    #[arg(long, default_value = "https://agent.aussierobots.com.au/mcp")]
    agent_url: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Call an MCP tool and print result to stdout
    Call {
        /// Tool name (e.g. account_devices, device_location)
        tool_name: String,

        /// Tool parameters as key=value pairs
        #[arg(short, long = "param", value_name = "KEY=VALUE")]
        params: Vec<String>,

        /// Output format
        #[arg(short, long, default_value = "json", value_parser = ["json", "yaml", "toml", "toon"])]
        output: String,
    },
    /// Clear stored OAuth tokens and log out
    Logout,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("gps_trust_mcp_tui=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();

    let use_oauth = cli.oauth && !cli.no_oauth;

    match cli.command {
        Some(Command::Call { tool_name, params, output }) => {
            call::run_call(
                &tool_name,
                &params,
                &output,
                cli.api_key,
                use_oauth,
                &cli.user_url,
                &cli.agent_url,
            )
            .await
        }
        Some(Command::Logout) => {
            auth::token_store::TokenStore::delete()?;
            let logout_url = format!("{}/logout", auth::oauth::AUTH_BASE);
            eprintln!("Clearing server session...");
            let _ = open::that(&logout_url);
            eprintln!("Logged out. Tokens and session cleared.");
            Ok(())
        }
        None => run_tui(cli.api_key, use_oauth, cli.user_url, cli.agent_url).await,
    }
}

async fn run_tui(
    api_key: Option<String>,
    use_oauth: bool,
    user_url: String,
    agent_url: String,
) -> anyhow::Result<()> {
    // --- Phase 1: Build credentials ---
    let auth_manager = AuthManager::new(api_key, use_oauth, user_url.clone(), agent_url.clone());

    eprintln!("Authenticating...");
    let session = auth_manager
        .authenticate()
        .await
        .context("authentication failed")?;

    // --- Phase 2: Connect MCP servers + bootstrap identity ---
    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<Action>();

    let mut mcp_manager = McpManager::new(&session, &user_url, &agent_url)
        .context("failed to create MCP manager")?;

    eprintln!("Connecting to MCP servers...");
    mcp_manager
        .connect_all(action_tx.clone())
        .await
        .context("failed to connect MCP servers")?;

    // Resolve identity on the already-connected User session
    let identity = mcp_manager
        .bootstrap_identity()
        .await
        .context("failed to bootstrap identity")?;
    eprintln!("Authenticated as {}", identity.display_name);

    // Update session with resolved identity
    let mut session = session;
    session.account_id = identity.account_id;
    session.display_name = identity.display_name;
    session.entity_type = identity.entity_type;

    // List tools from both servers
    let tools = mcp_manager
        .list_all_tools()
        .await
        .context("failed to list tools")?;
    eprintln!("Loaded {} tools", tools.len());

    // Wrap manager for shared access from background tasks
    let mcp = Arc::new(Mutex::new(mcp_manager));

    // --- Phase 3: Enter TUI ---
    let mut terminal = tui::init()?;
    let mut app = App::new();
    app.update(Action::AuthSuccess(session));
    app.update(Action::McpConnected(mcp::types::ServerIdentity::User));
    app.update(Action::McpConnected(mcp::types::ServerIdentity::Agent));
    app.set_tools(tools);

    let event_handler = EventHandler::new(action_tx.clone());
    event_handler.start();

    let mut tick_interval = tokio::time::interval(Duration::from_millis(250));

    // Initial render
    terminal.draw(|f| ui::render(f, &mut app))?;

    loop {
        tokio::select! {
            Some(action) = action_rx.recv() => {
                // Let app reducer handle the action first
                app.update(action.clone());

                // Check if app wants to execute a tool
                if app.execute_requested {
                    app.execute_requested = false;

                    let request = if let Some(ref form) = app.form_state {
                        if !form.missing_required().is_empty() {
                            None // Required fields missing, stay on form
                        } else {
                            build_tool_call_request(&app)
                        }
                    } else {
                        // No form — build request directly from selected tool
                        app.selected_tool().map(|entry| ToolCallRequest {
                            server: entry.server,
                            tool_name: entry.tool.name.clone(),
                            arguments: serde_json::json!({}),
                        })
                    };

                    if let Some(request) = request {
                        let tool_name = request.tool_name.clone();
                        let mut rs = ResultState::new();
                        rs.tool_name = Some(tool_name);

                        app.form_state = None;
                        app.result_state = Some(rs);
                        app.active_task = None;
                        app.input_mode = app::InputMode::Normal;
                        app.focus = app::PanelFocus::Result;

                        let mcp = Arc::clone(&mcp);
                        let tx = action_tx.clone();
                        tokio::spawn(async move {
                            dispatch_tool_call(mcp, request, tx).await;
                        });
                    }
                }

                // Check if app wants to reconnect
                if app.reconnect_requested {
                    app.reconnect_requested = false;
                    let mcp = Arc::clone(&mcp);
                    let tx = action_tx.clone();
                    tokio::spawn(async move {
                        let mut manager = mcp.lock().await;
                        match manager.reconnect_all(tx.clone()).await {
                            Ok(()) => {
                                info!("Reconnected to MCP servers");
                                match manager.list_all_tools().await {
                                    Ok(tools) => {
                                        let _ = tx.send(Action::ToolsLoaded(tools));
                                    }
                                    Err(e) => warn!(error = %e, "Failed to list tools after reconnect"),
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "Reconnect failed");
                            }
                        }
                    });
                }

                // Side effects that need async (MCP calls)
                match &action {
                    Action::McpToolsRefreshed(_) => {
                        let mcp = Arc::clone(&mcp);
                        let tx = action_tx.clone();
                        tokio::spawn(async move {
                            let manager = mcp.lock().await;
                            match manager.list_all_tools().await {
                                Ok(tools) => {
                                    let _ = tx.send(Action::ToolsLoaded(tools));
                                }
                                Err(e) => {
                                    warn!(error = %e, "Failed to refresh tools");
                                }
                            }
                        });
                    }
                    _ => {}
                }

                if app.should_quit {
                    break;
                }
            }
            _ = tick_interval.tick() => {
                terminal.draw(|f| ui::render(f, &mut app))?;
            }
        }
    }

    // --- Cleanup ---
    tui::restore()?;

    // Best-effort disconnect
    let manager = mcp.lock().await;
    let _ = manager.disconnect_all().await;

    // Handle logout if requested
    if app.logout_requested {
        auth::token_store::TokenStore::delete()?;
        let logout_url = format!("{}/logout", auth::oauth::AUTH_BASE);
        let _ = open::that(&logout_url);
        eprintln!("Logged out. Tokens and session cleared.");
    }

    Ok(())
}

/// Build a ToolCallRequest from the current form state.
fn build_tool_call_request(app: &App) -> Option<ToolCallRequest> {
    let form = app.form_state.as_ref()?;
    let tool_entry = app.selected_tool()?;

    let arguments = assemble_args(&form.fields);

    Some(ToolCallRequest {
        server: tool_entry.server,
        tool_name: form.tool_name.clone(),
        arguments,
    })
}

/// Dispatch a tool call in the background and send the result back.
async fn dispatch_tool_call(
    mcp: Arc<Mutex<McpManager>>,
    request: ToolCallRequest,
    tx: mpsc::UnboundedSender<Action>,
) {
    let tool_name = request.tool_name.clone();
    let server = request.server;

    info!(tool = %tool_name, server = %server, "Dispatching tool call");

    let manager = mcp.lock().await;
    match manager.call_tool(request).await {
        Ok(result) => {
            info!(tool = %tool_name, "Tool call completed");
            let _ = tx.send(Action::McpToolResult(Box::new(result)));
        }
        Err(e) => {
            error!(tool = %tool_name, error = %e, "Tool call failed");
            let _ = tx.send(Action::McpToolResult(Box::new(
                turul_mcp_protocol::CallToolResult {
                    content: vec![turul_mcp_protocol::ContentBlock::Text {
                        text: format!("Error: {e}"),
                        annotations: None,
                        meta: None,
                    }],
                    is_error: Some(true),
                    structured_content: None,
                    meta: None,
                },
            )));
        }
    }
}
