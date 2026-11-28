#!/usr/bin/env bash
# Build and test the Kotlin library, OpenUICompose (kotlin/OpenUICompose):
#
#   1. the core built for this machine and for Android (cargo-ndk), and the
#      Kotlin binding generated from crates/openui-ffi;
#   2. the generated file checked: scripts/check-runtime-binding-api.py's
#      Kotlin assertions, and every declaration hidden (internal) by
#      scripts/kotlin-binding.py, so none of it is the library's public API;
#   3. the licence rule's self-test; the licence of every library that
#      ships, of every library only the tests use, and of the spec's
#      starting libraries not added yet (Coil, Vico); and, after the release
#      build, that no test-only library's classes are in the .aar;
#   4. the JVM tests, on the core built for this machine;
#   5. the release build of the library;
#   6. with --device or --serial (or ANDROID_SERIAL set), the instrumented
#      tests on that emulator or phone. Without one they are skipped, and
#      the script says so in its last lines: it never picks a device itself.
#
#   scripts/kotlin-check.sh [--device] [--serial SERIAL] [--abi arm64-v8a|x86_64]
#
# Needs the Android SDK and NDK, cargo-ndk and the ABI's Rust target
# (aarch64-linux-android by default; x86_64-linux-android for GitHub's
# emulator), Java 17 or later, Python 3, and Gradle 9.4 (GRADLE, or the
# wrapper distribution in ~/.gradle), which fetches the libraries the first
# time. Not part of scripts/check.sh.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

device=false
serial="${ANDROID_SERIAL:-}"
abi="arm64-v8a"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --device) device=true; shift ;;
    --serial) serial="$2"; device=true; shift 2 ;;
    --abi) abi="$2"; shift 2 ;;
    *) echo "usage: scripts/kotlin-check.sh [--device] [--serial SERIAL] [--abi arm64-v8a|x86_64]" >&2; exit 2 ;;
  esac
done
[[ -z "$serial" ]] || device=true

sdk="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_HOME="$sdk"
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  ANDROID_NDK_HOME="$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1)"
  export ANDROID_NDK_HOME
fi
target="${CARGO_TARGET_DIR:-$ROOT/target}"
binding="$target/kotlin-binding"
case "$abi" in
  arm64-v8a) triple=aarch64-linux-android ;;
  x86_64) triple=x86_64-linux-android ;;
  *) echo "kotlin-check.sh: unknown ABI $abi" >&2; exit 2 ;;
esac

echo "Building the core for this machine"
cargo build -q -p openui-ffi --lib
host_dir="$target/debug"

echo "Generating the Kotlin binding"
# The generator's build has other features, so it has its own folder: its
# library must not replace the one the JVM tests load.
gen="$target/uniffi-bindgen"
CARGO_TARGET_DIR="$gen" cargo build -q -p openui-ffi --features bindgen
gen_lib="$gen/debug/libopenui_ffi.dylib"
[[ -f "$gen_lib" ]] || gen_lib="$gen/debug/libopenui_ffi.so"
raw="$target/kotlin-generated"
rm -rf "$raw" "$binding/kotlin"
"$gen/debug/uniffi-bindgen" generate "$gen_lib" --language kotlin --no-format \
  --config kotlin/uniffi.toml --out-dir "$raw" >/dev/null
hidden="$binding/kotlin/dev/openui/compose/ffi/openui_ffi.kt"
python3 scripts/kotlin-binding.py "$raw/dev/openui/compose/ffi/openui_ffi.kt" "$hidden"

echo "Checking the generated binding"
python3 scripts/kotlin-binding.py --self-test -q
python3 scripts/kotlin-binding.py --check "$hidden"
python3 scripts/check-runtime-binding-api.py --kotlin "$hidden"

echo "Building the core for $abi (NDK $(basename "$ANDROID_NDK_HOME"))"
cargo ndk -t "$abi" build -q --release -p openui-ffi --lib
rm -rf "$binding/jniLibs"
mkdir -p "$binding/jniLibs/$abi"
cp "$target/$triple/release/libopenui_ffi.so" "$binding/jniLibs/$abi/"

gradle="${GRADLE:-$(ls "$HOME"/.gradle/wrapper/dists/gradle-9.*-bin/*/gradle-9.*/bin/gradle 2>/dev/null | sort -V | tail -1)}"
[[ -x "$gradle" ]] || { echo "kotlin-check.sh: no Gradle 9 (set GRADLE)" >&2; exit 1; }
tasks=(:OpenUICompose:checkLicencesSelfTest :OpenUICompose:checkLicences :OpenUICompose:checkCandidateLicences
       :OpenUICompose:testDebugUnitTest :OpenUICompose:assembleRelease :OpenUICompose:checkTestOnlyNotShipped)
if $device; then
  adb=("$sdk/platform-tools/adb")
  [[ -z "$serial" ]] || adb+=(-s "$serial")
  "${adb[@]}" get-state >/dev/null || { echo "kotlin-check.sh: no device${serial:+ $serial}" >&2; exit 1; }
  [[ -z "$serial" ]] || export ANDROID_SERIAL="$serial"
  tasks+=(:OpenUICompose:connectedDebugAndroidTest)
fi

echo "Running Gradle: ${tasks[*]}"
# --continue: a failing task (a licence, say) doesn't stop the others, so
# one run reports every failure; the script still fails.
"$gradle" --no-daemon --continue -p kotlin -Popenui.bindingDir="$binding" -Popenui.hostLibDir="$host_dir" "${tasks[@]}"

if $device; then
  echo "Kotlin check passed, with the instrumented tests on $("${adb[@]}" shell getprop ro.product.model | tr -d '\r') ($abi)"
else
  echo
  echo "!!! SKIPPED: the instrumented tests (kotlin/OpenUICompose/src/androidTest) did not run."
  echo "!!! Only the JVM tests, the binding checks, the licence checks and the build passed."
  echo "!!! Run again with --device (one device attached) or --serial SERIAL to run them."
fi
