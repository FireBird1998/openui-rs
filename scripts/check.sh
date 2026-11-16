#!/usr/bin/env bash
# Formatting, lints, docs and tests (doc examples included), then a build on
# the oldest Rust the crates claim. Needs only a Rust toolchain: no Bun, no
# network.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
python3 scripts/check-runtime-binding-api.py
python3 scripts/test-fuzz-child.py
# Every test binary runs under an allocation cap (TESTING.md): one declared
# without the counting allocator would run uncapped.
if find crates -path '*/tests/*/main.rs' -print -quit | grep -q .; then
  echo "check.sh: nested test binaries are forbidden; use tests/<name>.rs" >&2
  exit 1
fi
for f in crates/*/tests/*.rs; do
  grep -q 'static ALLOCATOR: test_alloc::Allocator' "$f" \
    || { echo "check.sh: $f declares no test_alloc allocator (see TESTING.md)" >&2; exit 1; }
done
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -q -p openui-lang -p openui-runtime -p openui-session -p openui-ffi -p openui-wasm
cargo test --all --no-fail-fast

# The workspace's rust-version. The toolchain is named exactly and rustup's
# auto-install is off: `cargo +1.88` once made rustup install a second copy.
msrv="1.88.0"
if rustup toolchain list 2>/dev/null | grep -q "^$msrv-"; then
  RUSTUP_AUTO_INSTALL=0 CARGO_TARGET_DIR=target/msrv cargo "+$msrv" check -q --workspace --all-targets
else
  echo "check.sh: Rust $msrv isn't installed, so the build on it is skipped (rustup toolchain install $msrv)" >&2
fi
