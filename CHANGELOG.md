# Changelog

## 0.4.6

- `--paste-login`: OAuth login with no localhost listener or browser launch, for ssh sessions where the browser's `127.0.0.1:19876` redirect cannot reach the machine. gttui prints the authorize URL; paste the redirected URL (or the bare `code=` value) back. A pasted state must match. One paste per server audience, as with the browser flow. Parser unit-tested (state check negated once).

## 0.4.5

- Test for the 2026-only guard: an in-process hand-written HTTP stub that fails `server/discover` with `-32601` and answers `initialize` at 2025-11-25; `connect` must return `server negotiated 2025-11-25, gttui speaks 2026-07-28 only`. Negated once (guard disabled → red).

## 0.4.4

- Deleted the `turul-mcp-server-legacy` (`=0.3.47`) dev-dependency, its in-process fixture and the 2025-lane test: the whole fleet now runs `turul-mcp-server` 0.4.6 and answers `server/discover` (agent-mcp 0.3.9 and sv-track 0.13.17 were the last two, live 12 Nov).
- `turul-mcp-derive` 0.4.5 treats a dev-only `turul-mcp-server` as the macro root, so the `turul-mcp-protocol` and `turul-mcp-builders` dev-deps are dropped.
- gttui is 2026-07-28 only: deleted the 2025-lane branches in `src/mcp/client.rs` (stream handler, session capabilities, `lane` field). `connect` now errors if the server negotiates anything but 2026-07-28.
- ADR-0001 rev 2.2.

## 0.4.3

- Dependencies: `turul-mcp-client` 0.4.7, `toml` 1.2, `dirs` 7.0, `toon-format` 0.7. Dev-deps resolve to turul-mcp-server 0.4.6, derive 0.4.4, builders 0.4.2, protocol 0.4.1, turul-rpc 0.2.5.
- Removed the never-read `progress` and `total` fields of `Action::McpProgress`.
- CLI subcommands take a `call::Connection` instead of five separate connection arguments (clears the `too_many_arguments` lints). All other clippy lints fixed; `clippy -D warnings` is clean.
- Held: the dev-dependency `turul-mcp-server-legacy` (`=0.3.47`) and its test fixture stay. They reproduce the server/discover `-32001` answer that guards the 2025-11-25 lane, and agent-mcp is not yet confirmed deployed on 0.4.6. Removal is its own patch once it is.
- The test-only `turul-mcp-derive`, `-builders` and `-protocol` dev-deps stay: derive 0.4.4 reads only `[dependencies]` for turul-mcp-server, so the prelude alone does not satisfy the macro.
