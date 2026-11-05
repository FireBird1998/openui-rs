#!/usr/bin/env bash
# Run the core's test suite on Android: cross-compiles the test binaries with
# cargo-ndk (64-bit ARM unless --abi says otherwise), pushes them to the device
# with the conformance data, and runs each there with live diffs off and the
# depth tests on a 512 KB thread stack. Needs a running emulator (or a phone
# over adb), the Android SDK and NDK, cargo-ndk, and the ABI's Rust target
# (aarch64-linux-android for arm64-v8a, x86_64-linux-android for x86_64, which
# is what GitHub's emulator runs: .github/workflows/android.yml). Not part of
# scripts/check.sh, and hunts don't run it.
#
#   scripts/android-test.sh [--serial SERIAL] [--abi arm64-v8a|x86_64]
#
# Two test binaries stay on the host: `report` checks CONFORMANCE.md in the
# source tree, and `cli` runs the host's command-line binary. Neither is about
# the platform.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

serial=""
abi="arm64-v8a"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --serial) serial="$2"; shift 2 ;;
    --abi) abi="$2"; shift 2 ;;
    *) echo "usage: scripts/android-test.sh [--serial SERIAL] [--abi arm64-v8a|x86_64]" >&2; exit 2 ;;
  esac
done

sdk="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
adb="$sdk/platform-tools/adb"
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  ANDROID_NDK_HOME="$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1)"
  export ANDROID_NDK_HOME
fi
[[ -x "$adb" ]] || { echo "android-test.sh: no adb at $adb" >&2; exit 1; }
[[ -d "$ANDROID_NDK_HOME" ]] || { echo "android-test.sh: no NDK (set ANDROID_NDK_HOME)" >&2; exit 1; }
device=("$adb")
[[ -z "$serial" ]] || device+=(-s "$serial")
"${device[@]}" get-state > /dev/null || { echo "android-test.sh: no device; start an emulator first" >&2; exit 1; }

echo "Building the test binaries for $abi (NDK $(basename "$ANDROID_NDK_HOME"))"
# `cargo ndk` reads cargo's JSON messages itself and doesn't pass them on;
# `cargo ndk-test` does, but exits 0 whatever cargo did, so the build's
# errors are looked for in its log.
build_log="target/android-build.log"
mkdir -p target
binaries="$(cargo ndk-test -t "$abi" -q --no-run -p openui-lang --message-format=json-render-diagnostics 2> "$build_log" |
  jq -r 'select(.reason == "compiler-artifact" and .executable != null and .profile.test) | "\(.target.kind[0]):\(.executable)"')"
if grep -q '^error' "$build_log" || [[ -z "$binaries" ]]; then
  tail -20 "$build_log" >&2
  echo "android-test.sh: the build failed (log in $build_log)" >&2
  exit 1
fi

dir=/data/local/tmp/openui-rs
echo "Pushing the conformance data to $dir"
"${device[@]}" shell "rm -rf $dir && mkdir -p $dir/bin"
"${device[@]}" push -q conformance "$dir/conformance" > /dev/null

failed=()
ran=0
for entry in $binaries; do
  # kind:path. The library's and the binary's unit tests share the crate's
  # name, so those two are told apart by kind.
  kind="${entry%%:*}"
  binary="${entry#*:}"
  name="$(basename "$binary")"
  test_name="${name%-*}"
  [[ "$kind" == test ]] || test_name="$test_name-$kind"
  case "$test_name" in
    report | cli) echo "skip $test_name (host only)"; continue ;;
  esac
  "${device[@]}" push -q "$binary" "$dir/bin/$name" > /dev/null
  echo "run  $test_name"
  if ! "${device[@]}" shell "cd $dir && OPENUI_CONFORMANCE_DIR=$dir/conformance OPENUI_NO_LIVE_DIFFS=1 DEPTH_STACK_KB=512 ./bin/$name -q 2>&1; echo \"exit \$?\"" |
    tee "target/android-$test_name.log" | tail -2 | grep -q '^exit 0'; then
    failed+=("$test_name")
  fi
  ran=$((ran + 1))
done

echo
echo "On $("${device[@]}" shell getprop ro.product.model | tr -d '\r') (Android $("${device[@]}" shell getprop ro.build.version.release | tr -d '\r'), $abi): $ran test binaries, ${#failed[@]} failed"
if [[ ${#failed[@]} -gt 0 ]]; then
  echo "Failed: ${failed[*]} (logs in target/android-<test>.log)" >&2
  exit 1
fi
