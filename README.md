# gt-ui — GPS Trust MCP Terminal UI

Terminal UI and CLI for interacting with GPS Trust MCP servers. Connects to both User MCP and Agent MCP servers simultaneously, providing an interactive tool browser and a scriptable `call` command for automation.

## Features

- **Dual MCP server connection** — User (26 tools) and Agent (25 tools) with live status indicators
- **OAuth 2.1 + PKCE** authentication (default) with API key fallback
- **Interactive TUI** — tool browser, schema-driven parameter forms, structured result view
- **CLI tool calling** — `gt-ui call <tool> -p key=value` with JSON output to stdout
- **Session management** — `gt-ui logout` clears local tokens and server session
- **Reconnect** — `r` key rebuilds MCP connections without restarting
- **Vim-style navigation** — `j`/`k`, `/` filter, `Tab` focus cycling

## Install

```bash
# From source
cargo install --path .

# Or build without installing
cargo build --release
# Binary at target/release/gt-ui
```

## Quick Start

```bash
# Interactive TUI (OAuth login — opens browser on first run)
gt-ui

# Interactive TUI with API key
gt-ui --api-key <key>

# Call a tool directly (JSON to stdout)
gt-ui call account_devices

# Call with parameters
gt-ui call device_location -p device_id=D#018f9ed3c...

# Logout (clears tokens + server session)
gt-ui logout
```

## Commands

| Command | Description |
|---------|-------------|
| *(none)* | Launch interactive TUI |
| `call <tool> [-p key=value]...` | Execute a tool, print JSON to stdout |
| `logout` | Clear stored OAuth tokens and server session |

## CLI Tool Calling

The `call` subcommand executes a single MCP tool and prints the JSON result to stdout. Useful for scripting, piping to `jq`, or integrating with other tools.

```bash
gt-ui call <TOOL_NAME> [-p KEY=VALUE]...
```

### Parameter Types

Values are auto-detected: numbers, booleans, and JSON objects parse automatically. Everything else is treated as a string.

```bash
# String (default)
gt-ui call account_robot_devices -p robot_id=R#018f9e85c9097ece9eee7600c26f873e

# Numbers
gt-ui call lon_lat_to_geohash -p latitude=-33.8688 -p longitude=151.2093 -p precision=9

# Booleans
gt-ui call list_agents_for_entity -p entity_id=S#abc -p enabled_only=true

# Multiple parameters
gt-ui call device_location_history -p device_id=D#001 -p limit=100
```

### Piping and Scripting

All log/status output goes to stderr, keeping stdout clean for JSON:

```bash
# Pretty-print with jq
gt-ui call account_devices | jq .

# Extract specific fields
gt-ui call account_devices | jq '.devices[].deviceName'

# Use in scripts
ROBOT_ID=$(gt-ui call account_robots | jq -r '.robots[0].robotId')
gt-ui call account_robot_devices -p robot_id=$ROBOT_ID

# Save to file
gt-ui call device_location -p device_id=D#001 > location.json
```

### Error Handling

Tool errors exit with a non-zero status code and print the error to stderr:

```bash
# Unknown tool
gt-ui call nonexistent_tool
# Error: tool 'nonexistent_tool' not found. Available tools: account_devices, ...

# Missing required parameter
gt-ui call account_robot_devices
# Error: tool returned error: ...
```

## Interactive TUI

Running `gt-ui` without a subcommand launches the interactive terminal UI.

### Layout

```
+──────────────────────────────────────────────────────────────────+
│ gt-ui  nick@aussierobots  [User:ok] [Agent:ok]  49 tools         │
+──────────────────────+───────────────────────────────────────────+
│  [U] Account Devices │  Account Devices  [U]                     │
│  [U] Account Robots  │  account_devices                          │
│> [U] Device Location │  read-only | idempotent                   │
│  [A] List Agents     │                                           │
│  ...                 │  Returns all devices account-wide...       │
│                      +───────────────────────────────────────────+
│                      │  [1] Structured   [2] Raw                  │
│  /filter_            │  ┌account_devices─────────────────────┐   │
│                      │  │ deviceCount  6                      │   │
│                      │  │ devices  (6 items)                  │   │
│                      │  │   deviceId  D#018f9ed3c...          │   │
│                      │  └────────────────────────────────────┘   │
+──────────────────────+───────────────────────────────────────────+
│ j/k: nav | Enter: open | /: filter | r: reconnect | q: quit     │
+──────────────────────────────────────────────────────────────────+
```

### Keybindings

#### Global
| Key | Action |
|-----|--------|
| `Tab` | Cycle focus between panes |
| `Esc` | Back / close current pane |
| `Ctrl+C` | Quit immediately |
| `r` | Reconnect MCP servers |
| `L` (shift) | Logout and quit |
| `q` | Quit |

#### Tool List (left pane)
| Key | Action |
|-----|--------|
| `j` / `k` | Navigate up/down |
| `Enter` | Execute (no params) or open form (has params) |
| `/` | Start filter (type to search, `Enter` to accept, `Esc` to clear) |

#### Parameter Form
| Key | Action |
|-----|--------|
| `j` / `k` | Navigate fields |
| `Enter` | Start editing selected field |
| `Space` | Toggle boolean field |
| `e` | Execute tool (validates required fields) |
| `Esc` | Back to detail view |

#### Result View
| Key | Action |
|-----|--------|
| `j` / `k` | Scroll up/down |
| `1` | Structured tab (friendly key-value view) |
| `2` | Raw tab (JSON pretty-print) |
| `Esc` | Close result |

## Authentication

### OAuth 2.1 (Default)

Uses authorization code flow with PKCE (S256). On first run:
1. Registers client via Dynamic Client Registration
2. Opens browser for login
3. Captures callback on `127.0.0.1:19876`
4. Stores refresh tokens at `~/.config/gps-trust/tokens.json` (0600 permissions)

Subsequent runs refresh tokens silently without browser interaction.

### API Key

Pass via `--api-key` flag or `GPS_TRUST_API_KEY` environment variable:

```bash
# Flag
gt-ui --api-key sk-your-key-here

# Environment variable
export GPS_TRUST_API_KEY=sk-your-key-here
gt-ui

# API key only (disable OAuth)
gt-ui --no-oauth --api-key sk-your-key-here
```

### Logout

Clears local refresh tokens and invalidates the server session:

```bash
gt-ui logout
```

In the TUI, press `L` (shift-L) to logout and quit. Next run will require a fresh browser login.

## Global Options

```
Options:
    --api-key <API_KEY>      API key [env: GPS_TRUST_API_KEY]
    --oauth                  Enable OAuth (default: true)
    --no-oauth               Disable OAuth (API key only)
    --user-url <URL>         User MCP server [default: https://gt.aussierobots.com.au/mcp]
    --agent-url <URL>        Agent MCP server [default: https://agent.aussierobots.com.au/mcp]
    -h, --help               Print help
```

## Requirements

- Rust 2024 edition
- Terminal with Unicode support
- For OAuth: browser access, auth server with localhost redirect URI in DCR allowlist

## Architecture

See [CLAUDE.md](CLAUDE.md) for codebase structure, patterns, and development guidelines.
