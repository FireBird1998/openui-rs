#!/usr/bin/env bash
# Builds the viewer, the Mac app in viewer/: the core's UniFFI binding as a
# static library, its Swift bindings, then the Swift package. Needs Xcode.
#
#   scripts/viewer.sh [rust|build|run|test] [--release]
#
#   rust   the binding library and its Swift bindings only (Xcode's ⌘R runs this first)
#   build  rust, then the app, wrapped as target/viewer/<config>/Viewer.app (the default)
#   run    build, then run the app with its log in this terminal
#   test   rust, then the viewer's Swift tests
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# Xcode runs this with a minimal PATH.
export PATH="$HOME/.cargo/bin:$PATH"

command="build"
profile="debug"
for arg in "$@"; do
  case "$arg" in
    rust|build|run|test) command="$arg" ;;
    --release) profile="release" ;;
    *) echo "usage: scripts/viewer.sh [rust|build|run|test] [--release]" >&2; exit 2 ;;
  esac
done

rust() {
  if [[ "$profile" == "release" ]]; then
    cargo build -q -p openui-ffi --lib --release
  else
    cargo build -q -p openui-ffi --lib
  fi

  # The generator reads the interface from a built library. It's built with
  # its own target directory so its extra features never touch the library
  # the app links.
  local gen="$ROOT/target/uniffi-bindgen"
  CARGO_TARGET_DIR="$gen" cargo build -q -p openui-ffi --features bindgen
  local out
  out="$(mktemp -d)"
  trap 'rm -rf "$out"' RETURN
  "$gen/debug/uniffi-bindgen" generate "$gen/debug/libopenui_ffi.dylib" \
    --language swift --out-dir "$out" >/dev/null

  mkdir -p viewer/Sources/OpenUIFFI "$out/headers"
  cp "$out/openui_ffi.swift" viewer/Sources/OpenUIFFI/openui_ffi.swift
  mv "$out/openui_ffiFFI.h" "$out/headers/"
  mv "$out/openui_ffiFFI.modulemap" "$out/headers/module.modulemap"
  rm -rf viewer/Frameworks/openui_ffiFFI.xcframework
  xcodebuild -create-xcframework \
    -library "target/$profile/libopenui_ffi.a" -headers "$out/headers" \
    -output viewer/Frameworks/openui_ffiFFI.xcframework >/dev/null
}

# The Swift package builds a bare executable; a bundle around it gives the
# app its identity (name, Dock icon, its own settings and Keychain items).
app() {
  swift build --package-path viewer -c "$profile"
  local bin app="target/viewer/$profile/Viewer.app"
  bin="$(swift build --package-path viewer -c "$profile" --show-bin-path)/Viewer"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS"
  cp viewer/App/Info.plist "$app/Contents/"
  cp "$bin" "$app/Contents/MacOS/Viewer"
  codesign --force --sign - "$app" 2>/dev/null
  echo "$app"
}

rust
case "$command" in
  rust) ;;
  build) app ;;
  run) "$(app | tail -1)/Contents/MacOS/Viewer" ;;
  test) swift test --package-path viewer ;;
esac
