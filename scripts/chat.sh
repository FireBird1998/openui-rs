#!/usr/bin/env bash
# Builds the chat app, the sandboxed Mac app in tools/chat/: the core's Swift
# binding (scripts/swift-binding.sh), then the Swift package. Needs Xcode.
#
#   scripts/chat.sh [rust|build|run|test] [--release]
#
#   rust   the binding library and its Swift bindings only
#   build  rust, then the app, wrapped and signed as target/chat/<config>/Chat.app (the default)
#   run    build, then open the app
#   test   rust, then the chat app's Swift tests
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

command="build"
profile="debug"
for arg in "$@"; do
  case "$arg" in
    rust|build|run|test) command="$arg" ;;
    --release) profile="release" ;;
    *) echo "usage: scripts/chat.sh [rust|build|run|test] [--release]" >&2; exit 2 ;;
  esac
done

rust() {
  if [[ "$profile" == "release" ]]; then
    scripts/swift-binding.sh --release
  else
    scripts/swift-binding.sh
  fi
}

# The Swift package builds a bare executable; the bundle around it gives the
# app its identity (its Keychain item and sandbox container follow the
# bundle identifier) and carries the sandbox entitlements. It is signed ad
# hoc, for this Mac only: no paid developer account, no Keychain access
# group, no associated domains.
app() {
  swift build --package-path tools/chat -c "$profile"
  local bin app="target/chat/$profile/Chat.app"
  bin="$(swift build --package-path tools/chat -c "$profile" --show-bin-path)/Chat"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cp tools/chat/App/Info.plist "$app/Contents/"
  cp "$bin" "$app/Contents/MacOS/Chat"
  cp -R "$(dirname "$bin")/Chat_ChatKit.bundle" "$app/Contents/Resources/"
  codesign --force --sign - --entitlements tools/chat/App/Chat.entitlements "$app"
  echo "$app"
}

case "$command" in
  rust) rust ;;
  build) rust; app ;;
  run) rust; open "$(app | tail -1)" ;;
  test) rust; swift test --package-path tools/chat ;;
esac
