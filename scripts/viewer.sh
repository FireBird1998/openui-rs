#!/usr/bin/env bash
# Builds the viewer, the Mac app in tools/viewer/: the core's Swift binding
# (scripts/swift-binding.sh), then the Swift package. Needs Xcode.
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
    scripts/swift-binding.sh --release
  else
    scripts/swift-binding.sh
  fi
}

# The Swift package builds a bare executable; a bundle around it gives the
# app its identity (name, Dock icon, its own settings and Keychain items).
app() {
  swift build --package-path tools/viewer -c "$profile"
  local bin app="target/viewer/$profile/Viewer.app"
  bin="$(swift build --package-path tools/viewer -c "$profile" --show-bin-path)/Viewer"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS"
  cp tools/viewer/App/Info.plist "$app/Contents/"
  cp "$bin" "$app/Contents/MacOS/Viewer"
  codesign --force --sign - "$app" 2>/dev/null
  echo "$app"
}

rust
case "$command" in
  rust) ;;
  build) app ;;
  run) "$(app | tail -1)/Contents/MacOS/Viewer" ;;
  test) swift test --package-path tools/viewer ;;
esac
