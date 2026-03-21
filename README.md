# gt-ui — GPS Trust MCP Terminal UI

Terminal UI for interacting with GPS Trust MCP servers. Connects to both User MCP and Agent MCP servers simultaneously, providing tool browsing, parameter forms, and structured result viewing.

## Features

- **Dual MCP server connection** — User (26 tools) and Agent (25 tools) with status indicators
- **OAuth 2.1 + PKCE** authentication (default) with API key fallback
- **Tool browser** — filterable list with annotation titles, server prefix `[U]`/`[A]`
- **Schema-driven forms** — auto-generated from tool input schemas, managed fields hidden
- **Structured result view** — friendly key-value rendering with Raw JSON tab
- **Vim-style navigation** — `j`/`k`, `/` filter, `Tab` focus cycling

## Install

```bash
cargo install --path .
```

## Usage

```bash
# OAuth login (default — opens browser)
gt-ui

# API key authentication
gt-ui --api-key <key>

# API key only (no OAuth)
gt-ui --no-oauth --api-key <key>

# Custom server URLs
gt-ui --user-url https://gt.example.com/mcp --agent-url https://agent.example.com/mcp
```

## Keybindings

### Tool List
| Key | Action |
|-----|--------|
| `j` / `k` | Navigate up/down |
| `Enter` | Open parameter form |
| `/` | Start filter |
| `Tab` | Next pane |
| `q` | Quit |

### Parameter Form
| Key | Action |
|-----|--------|
| `j` / `k` | Navigate fields |
| `Enter` | Edit selected field |
| `Space` | Toggle boolean |
| `e` | Execute tool |
| `Esc` | Back to detail |

### Result View
| Key | Action |
|-----|--------|
| `j` / `k` | Scroll |
| `1` / `2` | Switch Structured/Raw tab |
| `Esc` | Close result |
| `Tab` | Next pane |

## Authentication

### OAuth 2.1 (Default)

Uses authorization code flow with PKCE (S256). On first run:
1. Registers client via Dynamic Client Registration
2. Opens browser for login
3. Captures callback on `127.0.0.1:19876`
4. Stores refresh tokens at `~/.config/gps-trust/tokens.json`

Subsequent runs refresh tokens silently without browser interaction.

### API Key

Pass via `--api-key` flag or `GPS_TRUST_API_KEY` environment variable. Bootstraps account identity via `entity_info` tool call.

## Requirements

- Rust 2024 edition
- Terminal with Unicode support
- For OAuth: browser access, auth server with localhost redirect URI in DCR allowlist

## Architecture

See [CLAUDE.md](CLAUDE.md) for codebase structure, patterns, and development guidelines.
