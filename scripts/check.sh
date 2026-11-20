#!/usr/bin/env bash
# Formatting, lints, docs and tests (doc examples included), the WebAssembly
# packages built, loaded and tested, then a build on the oldest Rust the crates
# claim. Needs only a Rust toolchain and no network: the WebAssembly step and
# the old-Rust build are skipped, with a note, when their tools are missing.
# OPENUI_REQUIRE_WASM=1 (CI sets it) turns the WebAssembly skip into a failure.
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

# The WebAssembly packages: scripts/wasm.sh builds both and loads them through
# the package guard (ADR 0007), then Bun runs the package tests. Rust tests
# can't see the guard, so a package that fails to load passes everything above.
wasm_missing=""
wasm_bindgen_want="$(scripts/wasm-bindgen-version.sh)"
wasm_bindgen_have="$(wasm-bindgen --version 2> /dev/null | awk '{ print $2 }' || true)"
if [[ "$wasm_bindgen_have" != "$wasm_bindgen_want" ]]; then
  wasm_missing="wasm-bindgen $wasm_bindgen_want isn't installed (found ${wasm_bindgen_have:-none}; cargo install wasm-bindgen-cli --version $wasm_bindgen_want --locked)"
elif [[ ! -d "$(rustc --print sysroot)/lib/rustlib/wasm32-unknown-unknown" ]]; then
  wasm_missing="the wasm32-unknown-unknown target isn't installed (rustup target add wasm32-unknown-unknown)"
elif ! command -v bun > /dev/null; then
  wasm_missing="Bun isn't installed"
fi
if [[ -z "$wasm_missing" ]]; then
  scripts/wasm.sh
  bun --no-install test crates/openui-wasm/js
elif [[ "${OPENUI_REQUIRE_WASM:-}" == 1 ]]; then
  echo "check.sh: $wasm_missing, and OPENUI_REQUIRE_WASM=1 forbids skipping the WebAssembly packages" >&2
  exit 1
else
  echo "check.sh: $wasm_missing, so the WebAssembly packages are not built or tested" >&2
fi

# The workspace's rust-version. The toolchain is named exactly and rustup's
# auto-install is off: `cargo +1.88` once made rustup install a second copy.
msrv="1.88.0"
if rustup toolchain list 2>/dev/null | grep -q "^$msrv-"; then
  RUSTUP_AUTO_INSTALL=0 CARGO_TARGET_DIR=target/msrv cargo "+$msrv" check -q --workspace --all-targets
else
  echo "check.sh: Rust $msrv isn't installed, so the build on it is skipped (rustup toolchain install $msrv)" >&2
fi
