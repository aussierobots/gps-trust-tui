# GPS Trust MCP TUI (gttui)

Terminal UI for interacting with GPS Trust User MCP and Agent MCP servers via `turul-mcp-client 0.3.x`.

## Build Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo run --bin gttui           # Run (OAuth default)
cargo run --bin gttui -- --help # CLI help
cargo run --bin gttui -- --api-key <key>  # API key mode
cargo run --bin gttui -- --no-oauth --api-key <key>  # API key only
cargo run --bin gttui -- call account_devices        # CLI tool call
cargo test                                           # Run tests
```

## Architecture

```
src/
  main.rs              — CLI (clap), tokio runtime, subcommand dispatch, TUI event loop
  call.rs              — CLI `call` subcommand: parse_params, extract_json, run_call
  app.rs               — App state, update() reducer, handle_char() routing
  event.rs             — EventHandler (crossterm → mpsc), bracketed paste
  action.rs            — Action enum (all user + system + MCP actions)
  tui.rs               — Terminal init/restore (alternate screen, bracketed paste)
  auth/
    mod.rs             — AuthManager: mode selection (OAuth default, API key opt-in)
    oauth.rs           — OAuth 2.1 authorization_code + PKCE, localhost callback
    api_key.rs         — API key auth via entity_info bootstrap
    session.rs         — AuthSession: per-server credentials, headers_for()
    token_store.rs     — ~/.config/gps-trust/tokens.json persistence (0600)
  mcp/
    mod.rs             — McpManager: dual-client lifecycle, managed field injection
    client.rs          — Single-server wrapper: paginated list, call_tool
    types.rs           — ServerIdentity, ToolEntry, ManagedFieldsPolicy
    notifications.rs   — Progress + list_changed dispatch, coalescer
  ui/
    mod.rs             — Top-level render() dispatch, right panel layout
    layout.rs          — Panel arrangement (status, tools, detail, progress, footer)
    status_bar.rs      — Connection status, identity, mode badge
    tool_browser.rs    — Tool list with filter + detail pane
    tool_form.rs       — Schema-driven parameter form with managed fields
    result_view.rs     — Structured/Raw tabs, content block renderer
    task_view.rs       — Task status, progress bar
    login_view.rs      — Auth status display
```

## Key Patterns

- **All printable chars** flow through `FilterChar(c)` from the event handler. `handle_char()` in app.rs routes them based on mode + focus. No hardcoded key→action mappings for printable chars at the event level.
- **ManagedFieldsPolicy** injects `account_id` from the session into all tool call arguments. The field is hidden from forms.
- **Tool display names** use annotation titles when available, falling back to snake_case tool names.
- **Result view** has Structured (friendly key-value) and Raw (JSON) tabs, switchable with 1/2 keys.
- **Execute (`e` key)** only works from the Form pane. From Detail, Enter opens the form. Tools with no required user params can be executed directly via the form's `[e] Execute`.

## Versioning

- **Patch bump (0.x.Y)**: Bug fixes, UI tweaks, minor improvements
- **Minor bump (0.X.0)**: New features (new pane, new auth method, new MCP capability support)
- After making changes, bump version in `Cargo.toml` accordingly before committing

## Commit Guidelines

- Succinct messages, 50-72 chars subject line
- NO AI attribution (no "Generated with Claude", no "Co-Authored-By: Claude")
- Conventional commits: feat:, fix:, docs:, refactor:
- Imperative mood ("Add feature" not "Added feature")

## Servers

- User MCP: `https://gt.aussierobots.com.au/mcp` (26 tools)
- Agent MCP: `https://agent.aussierobots.com.au/mcp` (25 tools)
- Auth: `https://auth.aussierobots.com.au` (OAuth 2.1 + PKCE)

## Related Repos

- [gps-trust-auth](~/gps-trust-auth) — OAuth auth server (DCR allowlist includes localhost callback)
- [gps-trust-agent-mcp](~/gps-trust-agent-mcp) — Agent MCP server
- [gps-trust-mcp](~/gps-trust-mcp) — User MCP server
