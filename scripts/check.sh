#!/usr/bin/env bash
# Formatting, lints and tests. Needs only a Rust toolchain.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
