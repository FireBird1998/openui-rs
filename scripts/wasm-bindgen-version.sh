#!/usr/bin/env bash
# Prints the wasm-bindgen version Cargo.lock names: the wasm-bindgen CLI must
# be exactly this version. scripts/wasm.sh, scripts/check.sh and CI's install
# all read it here, so Cargo.lock is the only place it is written.
#
#   scripts/wasm-bindgen-version.sh
set -euo pipefail
cd "$(dirname "$0")/.."
version="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/[^0-9.]/, ""); print; exit }' Cargo.lock)"
if [[ -z "$version" ]]; then
  echo "wasm-bindgen-version.sh: Cargo.lock names no wasm-bindgen" >&2
  exit 1
fi
echo "$version"
