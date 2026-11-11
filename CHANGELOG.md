# Changelog

## 0.4.3

- Dependencies: `turul-mcp-client` 0.4.7, `toml` 1.2, `dirs` 7.0, `toon-format` 0.7. Dev-deps resolve to turul-mcp-server 0.4.6, derive 0.4.4, builders 0.4.2, protocol 0.4.1, turul-rpc 0.2.5.
- Removed the never-read `progress` and `total` fields of `Action::McpProgress`.
- CLI subcommands take a `call::Connection` instead of five separate connection arguments (clears the `too_many_arguments` lints). All other clippy lints fixed; `clippy -D warnings` is clean.
- Held: the dev-dependency `turul-mcp-server-legacy` (`=0.3.47`) and its test fixture stay. They reproduce the server/discover `-32001` answer that guards the 2025-11-25 lane, and agent-mcp is not yet confirmed deployed on 0.4.6. Removal is its own patch once it is.
- The test-only `turul-mcp-derive`, `-builders` and `-protocol` dev-deps stay: derive 0.4.4 reads only `[dependencies]` for turul-mcp-server, so the prelude alone does not satisfy the macro.
