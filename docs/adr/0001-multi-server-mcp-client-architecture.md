# ADR-0001: Multi-Server MCP Client Architecture

**Status**: Accepted

**Date**: 2026-05-30

**Deciders**: Nick Hortovanyi

**Technical Story**: Extend gttui beyond the User + Agent MCP servers to also reach the Space Data Knowledge Base and SV Track MCP servers, with access to each MCP restricted per entity.

## Context

`gttui` currently connects to exactly two MCP servers (User, Agent). The codebase is already ~80% a generic N-server harness — the hard parts scale untouched — but the leaf bindings are hardcoded to a 2-variant `ServerIdentity` enum and two named client fields. We want to add at least two more servers (Space Data KB at `sd.aussierobots.com.au`, SV Track at `st.aussierobots.com.au`) and resolve the already-half-referenced Particle Filter (`pf.aussierobots.com.au`).

The maintainer's stated goal is to **restrict access to each MCP per entity** ("a basic user cannot reach SV Track"), enforced via the auth server `~/gps-trust-auth`, not by the TUI.

Forces:
- **Auth boundary**: per-MCP access must be enforced server-side and be non-bypassable; the TUI is a client, never the security boundary.
- **Minimum code / no speculative paths** (project convention): generalize, don't rewrite; don't add per-server machinery the contracts don't require.
- **Heterogeneity**: do the target servers share one contract, or diverge enough to justify separate crates?
- **Robustness**: adding servers must not let one server's outage or denial break access to the others.

Ground truth established by reading `~/gps-trust-auth`, `~/gps-trust-mcp`, `~/gps-trust-agent-mcp` and by a live probe (all 2026-05-30):

1. **The auth server already issues per-audience tokens.** `DCR_AUDIENCE_SCOPE_POLICY` (`gps-trust-auth-server/src/handlers.rs:136-151`) already lists `sd` and `st` (both `mcp:read`), validates RFC 8707 `resource=` indicators, and binds a single `aud` per JWT. Tests already cover the SV Track audience.
2. **Each MCP server has its own AWS Lambda authorizer** (API Gateway REQUEST type, always-invoke) that validates JWT signature + issuer + that server's `aud`, accepts Bearer and/or `X-API-Key`, and derives `account_id`/`entity_type` from credential context into `x-authorizer-*` headers → `EntityContext`.
3. **Account context is never a tool argument.** No User/Agent/SV-Track/Space-Data/PF tool declares `account_id`; account scoping rides on the authorizer context (and `device_id` + ownership checks where relevant). A live probe confirmed both new servers **silently ignore** a stray `account_id` argument (identical output with and without it) — turul has no `additionalProperties: false`.
4. **Per-entity audience restriction does not exist yet.** `~/gps-trust-auth/FUTURE.md:27-41` documents it as a known PoC gap: any authenticated account can currently obtain a token for any listed audience. Closing it requires CIMD, separate per-resource issuers, or a DCR `requested_audiences` field — all **auth-server** work.

## Decision

**Extend the existing single-binary workspace to N servers; do not split into per-server crates.** Specifically:

1. Replace `enum ServerIdentity { User, Agent }` (`src/mcp/types.rs:7`) with a `ServerId` + `ServerConfig` registry (id, label, prefix, url, audience, scope policy, `is_identity_provider`, `injects_account_id`). *(Implemented in slice 1 as `ServerId(Arc<ServerConfig>)` — the id handle wraps its own config so display/url resolve without a registry lookup at each call site; equality and hashing are by a stable `key` string. `injects_account_id` is deferred until the probe-gated `ManagedFieldsPolicy` decision, per Decision §4.)* Replace `McpManager`'s named `user_client`/`agent_client` fields with `HashMap<ServerId, McpServerClient>`; every hardcoded 2-arm match and 2-element array (`mcp/mod.rs`, `auth/oauth.rs:41`, `auth/api_key.rs`, `app.rs:72`, `status_bar.rs:36`, `main.rs:217`) becomes iteration over the registry.
2. **Per-MCP access restriction is an auth-server responsibility.** gttui requests a resource-indicated token per audience and is a *fail-closed client*: a server for which no usable token is minted is marked `Unauthorized` and omitted — never reached with a header-less request. The TUI does not model or simulate the access policy.
   - **Rollout guard**: because per-entity audience gating does not exist in the auth server yet (every authenticated account can currently mint a token for any audience), the new servers MUST stay behind a config gate that **defaults off**. Enabling them for all users before `requested_audiences` lands would *broaden* access (every gttui user reaches SV Track / Space Data KB), which is the inverse of the stated goal. Reachability is opt-in until the auth-server restriction is in place.
3. **`bootstrap_identity()` stays pinned to the single `is_identity_provider` server (User).** In OAuth mode `account_id` already comes from the JWT `sub` at logon (`oauth.rs:95`); `entity_info` only enriches display name/type. The new servers expose no `entity_info` and must not be looped into bootstrap.
4. **Leave `ManagedFieldsPolicy` injection unchanged.** The probe shows `account_id` injection is inert and harmless on the new servers. A per-server `injects_account_id=false` flag is recorded as optional tidiness, not a correctness requirement.
5. **Activate the client-side audience-binding assertion** (the already-stored, currently `dead_code` `ServerCredentials.audience`, `session.rs:13`) as cheap defense-in-depth: before emitting a Bearer header, assert the credential's audience matches the target server's configured audience.

## Consequences

### Positive

- Adding a server becomes a registry/config entry plus deployment of that server's own Lambda authorizer; the TUI logic stops growing per server.
- One OAuth flow and one audience-keyed token store (`token_store.rs:12` is already `HashMap<String, _>`) serve all servers — no fragmentation.
- Fail-soft startup means one server's outage or denial no longer aborts the whole TUI.
- The audience assertion catches token mix-ups / routing bugs locally with a clear message instead of an opaque remote 403.

### Negative

- The registry refactor touches many files at once (transport, auth, UI, startup); large diff, though runtime-behavior-preserving with only User+Agent registered.
- The maintainer's actual goal (per-entity MCP restriction) is **blocked on external auth-server work** (`requested_audiences` / CIMD); this ADR cannot deliver it alone. Registering the new servers before that work lands broadens access rather than restricting it — hence the default-off config gate (Decision §2).
- Tool-name collisions appear at 3+ servers (`field_metadata` on User and SV Track; `list_data_sources` on both new servers), requiring a `--server` disambiguator in the CLI.

### Neutral

- The `account_id` contract is settled *as of 2026-05-30* by probe; it is not a permanent guarantee (see Implementation Notes).
- CLAUDE.md's "dual-client" framing and the 2-server lists become doc drift to correct in the same slice as the registry.

## Alternatives Considered

### Alternative 1: Separate crates / workspace member per MCP

**Description**: A Cargo workspace with `gttui-core`, `gttui-auth`, and a crate per server.

**Pros**: Compile-time isolation of per-server policy; independent versioning.

**Cons**: One binary, one event loop — a panic in any linked crate kills the TUI regardless, so the isolation is illusory. Forces the genuinely shared surface (`ToolEntry`, form state, the entire `ui/` tree, the `call.rs` CLI) across crate boundaries.

**Why Rejected**: All target servers share one contract (per-audience token + authorizer-derived account context); there is no third consumer and no independent release cadence. Over-engineering at this scale.

### Alternative 2: Enforce per-MCP access in the TUI

**Description**: Have gttui decide which servers an entity may reach.

**Pros**: Visible in one place.

**Cons**: A client-side gate is bypassable and duplicates policy that belongs to the auth server + Lambda authorizers.

**Why Rejected**: The auth server and per-server authorizers are the only trustworthy boundary; the TUI must remain a fail-closed client.

## Implementation Notes

Ordered, independently-shippable slices (version bumps per project convention — refactor = patch, new MCP capability = minor). **This numbering is canonical and supersedes any earlier draft numbering** — map commits to these slice numbers:

0. **Probe (done, 2026-05-30)** — confirmed new servers ignore stray `account_id`; no tool declares a scoped arg. **Re-run this probe whenever registering a new server**, since "ignored" is current behavior, not a contract guarantee.
1. **Registry**: `ServerIdentity` enum → `ServerId` + `ServerConfig`; `McpManager` named fields → client map (still only User+Agent registered; behavior-preserving). Update CLAUDE.md. *(patch)*
2. **Fail-soft startup** + distinct `Unauthorized` connection state; gate connection on credential presence. *(patch)*
3. **Activate audience-binding assertion** in `headers_for()`; first auth-layer tests. *(patch)*
4. **Config-driven registration** (`--server id=url` / `servers.toml`) + status-bar compaction + `--server` disambiguator for tool-name collisions. *(minor)*
5. **Register Space Data KB**, then **SV Track + resolve dangling `pf`** — each needs error-state + collision + test work, not data-only. **Gated default-off** (Decision §2): do not enable for all users until the auth-server `requested_audiences` restriction lands. Until then, registration is opt-in (config/feature flag) for development and testing only. *(minor)*
6. **Hygiene/deps**: `.DS_Store` → `.gitignore`, evaluate `serde_yml` → `serde_yaml`, backfill tests. *(patch)*

**Implementation status (2026-05-30):** slices 0–6 landed on `feat/multi-server-registry` (→ v0.3.1). Slice 6: `.gitignore` hygiene; `serde_yml` evaluated and **kept** (it is the maintained fork of the *archived* `serde_yaml` — migrating would regress); tests backfilled (21 → 34, first auth/registry coverage). Also fixed a pre-existing bug: the OAuth client requested only `mcp:read` for `gt` — scope now lives on `ServerConfig` (catalog) and matches the auth policy (`gt`/`agent`/`pf` read+write; `sv-track`/`space-data` read-only). Commit order was 1, 2, 4, 3, 5, 6 (independent slices). Slice 5 registers `pf`/`sv-track`/`space-data` as a **catalog with default URLs, opt-in via `--server <key>`** (not forced on the default fleet — a baseline account would otherwise make doomed token requests and show noise badges). Making them default-fleet is a deferred one-line follow-up. The auth-server per-account backfill is **live** (test accounts: `nickh`/`username1` positive; `co01`/`cmp01` negative — pending an interactive live run). Naming note: registration is `--server KEY=URL` (or bare `--server <key>`); the call/describe collision disambiguator is `--from <server>`.

**Post-merge fix (v0.3.2):** DCR clients are ephemeral on the auth server (a redeploy drops them), so a blindly-reused cached `client_id` failed with `invalid_client` in the browser and hung the callback listener. gttui now preflights the cached `client_id` against `/authorize` (no-redirect GET; `invalid_client` body ⇒ stale) and re-registers transparently. CIMD (`client_id_metadata_document_supported`) remains the tidier long-term path.

**External dependency (not this repo):** per-entity audience gating in `~/gps-trust-auth` (`requested_audiences` field is the smallest option) — now implemented as the per-account `mcpAudiences` allowlist (auth ADR-0005), live in prod. It is the actual gate for "basic user cannot reach SV Track".

**Security-sensitive assumption to monitor:** the TUI treats the JWT `sub` as a stable `account_id` (`A#…`, `canonical_subject`) across the single issuer `auth.aussierobots.com.au`. Any change to issuer topology (e.g. separate per-resource issuers) or `sub` semantics invalidates this coupling and the audience assertion.

## Protocol Version and the 2026-07-28 Spec (rev 2, 2026-10-20)

Read from `turul-mcp-client 0.4.2` source (`src/version.rs`, `src/client.rs`, `src/config.rs`), not its README. Status: **Proposed** until the implementation slice below lands.

**1. Negotiation.** The default build is bilingual (`client-bilingual`). With `ClientConfig.mcp_protocol_version = None` it sends `server/discover` (advertising 2026-07-28) and locks one version per connection via `classify_probe`: a valid result locks 2026-07-28; JSON-RPC `-32601` or `-32602`, or `-32022` whose `data.supported` lists 2025-11-25, falls back to `initialize` at 2025-11-25; every other outcome aborts with `NegotiationFailed` (any other JSON-RPC code, `-32022` with no list, any HTTP 4xx/5xx; HTTP 404/405 fall back only with `allow_legacy_gateway_fallback`, off). A server that answers 2025-11-25 is spoken to at 2025-11-25 only if it fails the probe with one of those fallback signals, or if the hint is `Some(V2025_11_25)`, which skips the probe and runs `initialize` directly.

Fleet (all five locked to `turul-mcp-server 0.3.47`; none of their `src/` mentions `server/discover` or 2026-07-28; all 2025-11-25 until the fleet plan moves them):

| Server | Repo | Protocol |
|---|---|---|
| User (`gt`) | `gps-trust-mcp` | 2025-11-25 |
| Agent (`agent`) | `gps-trust-agent-mcp` | 2025-11-25 |
| `pf` | `gps-trust-sv-pf-mcp` | 2025-11-25 |
| `sv-track` (`st`) | `gps-trust-sv-track` | 2025-11-25 |
| `space-data` (`sd`) | `gps-trust-space-data-mcp` | 2025-11-25 |

The current pin (`src/mcp/client.rs`: `Some(V2025_11_25)`) exists because these servers answer the unauthenticated-session `server/discover` probe with a custom `-32001` "Missing Mcp-Session-Id", which is not a fallback signal, so the client aborts. That error body is the recorded live observation from the 0.4 migration; **re-probe each server before unpinning it** (needs a token, so not done in this revision).

**2. Does 0.4 alone cover 2026-07-28?** It covers the client wire side: `server/discover`, per-request `_meta` (protocolVersion, clientInfo, clientCapabilities), `MCP-Protocol-Version` header, no `Mcp-Session-Id`, `subscriptions/listen`, per-request progress (`call_tool_with_progress`), MRTR input-required retries, and the Tasks extension behind the `ext-tasks` feature. It does not cover: (a) any server speaking 2026 (none does); (b) TUI code that assumes 2025 behaviour (item 3); (c) schema detail the public types cannot hold (2026 tool schemas are remapped best-effort; the full schema is `McpClient::tool_input_schema`); (d) answering server-initiated input requests, which the TUI would have to implement before declaring `elicitation`/`sampling`/`roots` (all off by default, and stay off). Caveat: coverage is taken from the crate's own code and comments; the spec text itself was not independently re-read in this revision.

**3. Connection and notification model on 2026-07-28.**

| Today (2025-11-25) | On 2026-07-28 | Replacement |
|---|---|---|
| `initialize` + `Mcp-Session-Id`; `session_info().server_capabilities` feeds `inspect_caps` | no session; the 2026 lane leaves `session_info().server_capabilities` unset (only state is set Active) | read `server_capabilities()` / `discovered_server()`; `inspect_caps` must source from them when negotiated 2026 |
| GET SSE listener (`stream_handler().on_notification`, `on_connection_lost`) carries `tools/list_changed` | listener is not started (no GET stream) | `subscriptions_listen({"toolsListChanged": true})`, held per server; dropping the stream cancels it. Same Lambda/API Gateway ~29s idle limit applies, so reconnect is routine, as `on_connection_lost` already treats it |
| `notifications/progress` over the GET stream | no stream | `call_tool_with_progress` per call, feeding the existing `Action::McpProgress` |
| 2025 tasks (`call_tool_with_task`, `get_task`, `cancel_task`, `ServerCaps.tasks_*`) | superseded by the Tasks extension | no TUI path uses these today (`#[allow(dead_code)]`); delete them in the implementing slice rather than port them. Enable `ext-tasks` only when a server offers it |

Tool calls, `list_tools_paginated`, bearer injection and `set_bearer` are version-agnostic from the TUI's side. Auth is unchanged.

**4. Smallest change that drops the pin without breaking item 1.** Removing the pin outright breaks all five servers today (probe aborts on `-32001`). The smallest correct change is to move the pin from a hard-coded constant in `McpServerClient::new` into the owning `ServerConfig` as an optional protocol hint per server: default `Some(V2025_11_25)` for every catalog entry, `None` (auto-negotiate) for a server once it is re-probed and answers `server/discover` or `-32601`. In the same slice: (i) `inspect_caps` reads the negotiated lane's capabilities source, (ii) notification setup branches on `negotiated_version()` (2025: stream handler as now; 2026: `subscriptions_listen`), (iii) progress uses `call_tool_with_progress` on 2026 only, (iv) the unused 2025 task wrappers are deleted. Before any server is unpinned, a test proves a 2025-11-25-only mock server still connects with hint `None` only if it returns a fallback signal, and that the default-pinned path is unchanged.

**Cross-repo and cost.** The design needs no change to any server or shared crate; moving a server off the pin is gated on that server's own fleet-plan upgrade. Monthly cost: $0.

## Related Decisions

- `~/gps-trust-mcp/docs/adr/001-middleware-based-authorization.md` — server-side authorization boundary this ADR relies on.
- `~/gps-trust-mcp/docs/adr/002-entity-context-session-injection.md` — how account context is derived from the authorizer, not tool args.
- `~/gps-trust-auth` `docs/plans/2026-03-08-per-resource-dcr-design.md` and `FUTURE.md` — the per-entity audience restriction gap.

## References

- `DCR_AUDIENCE_SCOPE_POLICY` — `~/gps-trust-auth/gps-trust-auth-server/src/handlers.rs:136-151`
- gttui hardcoded 2-server sites — `src/mcp/types.rs:7`, `src/mcp/mod.rs:26-218`, `src/auth/oauth.rs:41-44`, `src/auth/api_key.rs:14-26`, `src/app.rs:72`, `src/ui/status_bar.rs:36`, `src/main.rs:217`
- Live probe (2026-05-30): `gnss_list` and `list_data_sources` returned identical output with and without a stray `account_id` argument.

## Revision Log

- **rev 2 (2026-10-20)**: added "Protocol Version and the 2026-07-28 Spec" (per-server protocol hint replaces the global 2025-11-25 pin; 2026 connection/notification model). Proposed; implementation pending. No code change.
