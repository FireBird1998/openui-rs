#!/usr/bin/env bash
# Formatting, lints, docs and tests (doc examples included), then a build on
# the oldest Rust the crates claim. Needs only a Rust toolchain: no Bun, no
# network.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -q -p openui-lang
cargo test --all

# The workspace's rust-version. The toolchain is named exactly and rustup's
# auto-install is off: `cargo +1.88` once made rustup install a second copy.
msrv="1.88.0"
if rustup toolchain list 2>/dev/null | grep -q "^$msrv-"; then
  RUSTUP_AUTO_INSTALL=0 CARGO_TARGET_DIR=target/msrv cargo "+$msrv" check -q --workspace --all-targets
else
  echo "check.sh: Rust $msrv isn't installed, so the build on it is skipped (rustup toolchain install $msrv)" >&2
fi
