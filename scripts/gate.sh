#!/usr/bin/env bash
# Test gate: tests, then lint and format checks (this package only).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --no-fail-fast
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
