#!/usr/bin/env bash
# Builds the core's Swift binding for the OpenUINative package: the UniFFI
# binding as a static library in swift/OpenUINative/Frameworks/ and its Swift
# side in swift/OpenUINative/Sources/OpenUIFFI/. Both are generated, not
# committed. Needs Xcode.
#
#   scripts/swift-binding.sh [--release]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# Xcode runs this with a minimal PATH.
export PATH="$HOME/.cargo/bin:$PATH"

profile="debug"
case "${1:-}" in
  "") ;;
  --release) profile="release" ;;
  *) echo "usage: scripts/swift-binding.sh [--release]" >&2; exit 2 ;;
esac

if [[ "$profile" == "release" ]]; then
  cargo build -q -p openui-ffi --lib --release
else
  cargo build -q -p openui-ffi --lib
fi

# The generator reads the interface from a built library. It's built with
# its own target directory so its extra features never touch the library
# the app links.
gen="$ROOT/target/uniffi-bindgen"
CARGO_TARGET_DIR="$gen" cargo build -q -p openui-ffi --features bindgen
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
"$gen/debug/uniffi-bindgen" generate "$gen/debug/libopenui_ffi.dylib" \
  --language swift --out-dir "$out" >/dev/null

mkdir -p swift/OpenUINative/Sources/OpenUIFFI "$out/headers"
cp "$out/openui_ffi.swift" swift/OpenUINative/Sources/OpenUIFFI/openui_ffi.swift
mv "$out/openui_ffiFFI.h" "$out/headers/"
mv "$out/openui_ffiFFI.modulemap" "$out/headers/module.modulemap"
rm -rf swift/OpenUINative/Frameworks/openui_ffiFFI.xcframework
xcodebuild -create-xcframework \
  -library "target/$profile/libopenui_ffi.a" -headers "$out/headers" \
  -output swift/OpenUINative/Frameworks/openui_ffiFFI.xcframework >/dev/null
