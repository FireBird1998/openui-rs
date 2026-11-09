#!/usr/bin/env bash
# Build the Android demo app (android-demo/): the core and its UniFFI binding
# cross-compiled for 64-bit ARM, the Kotlin bindings generated from them, and
# a Compose app that streams the benchmark samples through the core a few
# characters at a time and draws each result natively. With --run, installs
# it on the device and starts it playing every sample in turn.
#
#   scripts/android-demo.sh [--run] [--serial SERIAL]
#
# Needs the Android SDK and NDK, cargo-ndk, the aarch64-linux-android Rust
# target, Java 17 or later, and Gradle 9 (GRADLE, or the wrapper distribution
# in ~/.gradle), which fetches the app's dependencies the first time. Not part
# of scripts/check.sh.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

run=false
serial=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) run=true; shift ;;
    --serial) serial="$2"; shift 2 ;;
    *) echo "usage: scripts/android-demo.sh [--run] [--serial SERIAL]" >&2; exit 2 ;;
  esac
done

sdk="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_HOME="$sdk"
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  ANDROID_NDK_HOME="$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1)"
  export ANDROID_NDK_HOME
fi
main=android-demo/app/src/main
libs="$main/jniLibs/arm64-v8a"
mkdir -p "$libs" "$main/assets/programs"

echo "Building the binding for arm64-v8a"
cargo ndk -t arm64-v8a build -q --release -p openui-ffi --lib
cp target/aarch64-linux-android/release/libopenui_ffi.so "$libs/"

echo "Generating the Kotlin bindings"
gen=target/uniffi-bindgen
CARGO_TARGET_DIR="$gen" cargo build -q -p openui-ffi --features bindgen
host_lib="$gen/debug/libopenui_ffi.dylib"
[[ -f "$host_lib" ]] || host_lib="$gen/debug/libopenui_ffi.so"
"$gen/debug/uniffi-bindgen" generate "$host_lib" --language kotlin --no-format \
  --config android-demo/uniffi.toml --out-dir "$main/java" >/dev/null

cp conformance/imported/cases/bench/*.oui "$main/assets/programs/"
cp conformance/imported/schemas/bench.json "$main/assets/"

echo "Building the app"
gradle="${GRADLE:-$(ls "$HOME"/.gradle/wrapper/dists/gradle-9.*-bin/*/gradle-9.*/bin/gradle 2>/dev/null | sort -V | tail -1)}"
[[ -x "$gradle" ]] || { echo "android-demo.sh: no Gradle 9 (set GRADLE)" >&2; exit 1; }
(cd android-demo && "$gradle" -q assembleDebug)
apk=android-demo/app/build/outputs/apk/debug/app-debug.apk
echo "Built $apk ($(du -h "$apk" | cut -f1))"

if $run; then
  device=("$sdk/platform-tools/adb")
  [[ -z "$serial" ]] || device+=(-s "$serial")
  "${device[@]}" install -r -t "$apk" >/dev/null
  "${device[@]}" shell am start -S -n dev.openui.demo/.MainActivity --ez autoplay true >/dev/null
  echo "Started on $("${device[@]}" shell getprop ro.product.model | tr -d '\r')"
fi
