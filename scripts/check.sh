#!/usr/bin/env bash
# Formatting, lints, docs and tests (doc examples included). Needs only a
# Rust toolchain: no Bun, no network.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -q -p openui-lang
cargo test --all
