#!/usr/bin/env bash
# Builds the core as WebAssembly (crates/openui-wasm) with its JavaScript
# glue: target/wasm-pkg/web for a browser or a bundler, target/wasm-pkg/node
# for Node and Bun. Needs the wasm32-unknown-unknown Rust target and the
# wasm-bindgen tool at the version Cargo.lock names
# (cargo install wasm-bindgen-cli --version <that> --locked).
#
#   scripts/wasm.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
want="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/[^0-9.]/, ""); print; exit }' Cargo.lock)"
have="$(wasm-bindgen --version 2> /dev/null | awk '{ print $2 }')"
if [[ "$have" != "$want" ]]; then
  echo "wasm.sh: wasm-bindgen $want is needed (found ${have:-none}): cargo install wasm-bindgen-cli --version $want --locked" >&2
  exit 1
fi
cargo build -q -p openui-wasm --release --target wasm32-unknown-unknown
module=target/wasm32-unknown-unknown/release/openui_wasm.wasm
for target in web nodejs; do
  out="target/wasm-pkg/${target%js}"
  rm -rf "$out"
  wasm-bindgen --target "$target" --out-dir "$out" "$module"
  python3 scripts/check-runtime-binding-api.py --wasm "$out/openui_wasm.d.ts"
  cat crates/openui-wasm/js/package-guards.js >> "$out/openui_wasm.js"
done
# Load the Node package once: the guard refuses to load when a generated
# public member (string- or symbol-keyed) is not covered by its surface table.
if command -v node > /dev/null; then
  node -e 'require(process.argv[1])' "$ROOT/target/wasm-pkg/node/openui_wasm.js"
elif command -v bun > /dev/null; then
  bun --no-install -e 'require(process.argv[1])' "$ROOT/target/wasm-pkg/node/openui_wasm.js"
else
  echo "wasm.sh: neither node nor bun found; the package guard's load check was skipped" >&2
fi
echo "Built target/wasm-pkg/web and target/wasm-pkg/node ($(wc -c < target/wasm-pkg/web/openui_wasm_bg.wasm | tr -d ' ') bytes of WebAssembly)"
