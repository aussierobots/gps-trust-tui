# gttui — GPS Trust MCP Terminal UI

Terminal UI and CLI for interacting with GPS Trust MCP servers. Connects to both User MCP and Agent MCP servers simultaneously, providing an interactive tool browser and a scriptable `call` command for automation.

## Features

- **Dual MCP server connection** — User (26 tools) and Agent (25 tools) with live status indicators
- **OAuth 2.1 + PKCE** authentication (default) with API key fallback
- **Interactive TUI** — tool browser, schema-driven parameter forms, structured result view
- **CLI tool calling** — `gttui call <tool> -p key=value` with JSON output to stdout
- **Session management** — `gttui logout` clears local tokens and server session
- **Reconnect** — `r` key rebuilds MCP connections without restarting
- **Vim-style navigation** — `j`/`k`, `/` filter, `Tab` focus cycling

## Install

### Prerequisites

- [Rust](https://rustup.rs/) (2024 edition, 1.85+)
- `~/.cargo/bin` must be in your `PATH`

To check:
```bash
echo $PATH | tr ':' '\n' | grep cargo
# Should show: /Users/<you>/.cargo/bin
```

If missing, add to your shell profile (`~/.zshrc`, `~/.bashrc`, etc.):
```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

### From source

```bash
# Clone and install
git clone git@github.com:aussierobots/gps-trust-tui.git
cd gps-trust-tui
cargo install --path .

# Verify
gttui --help
```

### Build without installing

```bash
cargo build --release
# Binary at target/release/gttui — copy it wherever you like
```

## Quick Start

```bash
# Interactive TUI (OAuth login — opens browser on first run)
gttui

# Interactive TUI with API key
gttui --api-key <key>

# Call a tool directly (JSON to stdout)
gttui call account_devices

# Call with parameters
gttui call device_location -p device_id=D#018f9ed3c...

# Output as YAML, TOML, or TOON
gttui call account_devices -o yaml

# Logout (clears tokens + server session)
gttui logout
```

## Commands

| Command | Description |
|---------|-------------|
| *(none)* | Launch interactive TUI |
| `call <tool> [-p key=value]... [-o format]` | Execute a tool, print result to stdout |
| `logout` | Clear stored OAuth tokens and server session |

## CLI Tool Calling

The `call` subcommand executes a single MCP tool and prints the result to stdout. Useful for scripting, piping to `jq`, or feeding into LLMs.

```bash
gttui call <TOOL_NAME> [-p KEY=VALUE]... [-o FORMAT]
```

### Output Formats

| Format | Flag | Description |
|--------|------|-------------|
| JSON | `-o json` (default) | Pretty-printed JSON, pipe-friendly with `jq` |
| YAML | `-o yaml` | Human-readable, good for config/review |
| TOML | `-o toml` | Config-friendly key-value format |
| TOON | `-o toon` | Token-efficient format for LLM prompts (~40% fewer tokens) |

```bash
gttui call account_devices                 # JSON (default)
gttui call account_devices -o yaml         # YAML
gttui call account_devices -o toml         # TOML
gttui call account_devices -o toon         # TOON (compact, LLM-friendly)
```

### Parameter Types

Values are auto-detected: numbers, booleans, and JSON objects parse automatically. Everything else is treated as a string.

```bash
# String (default)
gttui call account_robot_devices -p robot_id=R#018f9e85c9097ece9eee7600c26f873e

# Numbers
gttui call lon_lat_to_geohash -p latitude=-33.8688 -p longitude=151.2093 -p precision=9

# Booleans
gttui call list_agents_for_entity -p entity_id=S#abc -p enabled_only=true

# Multiple parameters
gttui call device_location_history -p device_id=D#001 -p limit=100
```

### Piping and Scripting

All log/status output goes to stderr, keeping stdout clean for JSON:

```bash
# Pretty-print with jq
gttui call account_devices | jq .

# Extract specific fields
gttui call account_devices | jq '.devices[].deviceName'

# Use in scripts
ROBOT_ID=$(gttui call account_robots | jq -r '.robots[0].robotId')
gttui call account_robot_devices -p robot_id=$ROBOT_ID

# Save to file
gttui call device_location -p device_id=D#001 > location.json
```

### Error Handling

Tool errors exit with a non-zero status code and print the error to stderr:

```bash
# Unknown tool
gttui call nonexistent_tool
# Error: tool 'nonexistent_tool' not found. Available tools: account_devices, ...

# Missing required parameter
gttui call account_robot_devices
# Error: tool returned error: ...
```

## Interactive TUI

Running `gttui` without a subcommand launches the interactive terminal UI.

### Layout

```
+──────────────────────────────────────────────────────────────────+
│ gttui  nick@aussierobots  [User:ok] [Agent:ok]  49 tools         │
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
gttui --api-key sk-your-key-here

# Environment variable
export GPS_TRUST_API_KEY=sk-your-key-here
gttui

# API key only (disable OAuth)
gttui --no-oauth --api-key sk-your-key-here
```

### Logout

Clears local refresh tokens and invalidates the server session:

```bash
gttui logout
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
