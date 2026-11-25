#!/usr/bin/env bash
# The Swift side's check: the binding rebuilt, then the OpenUINative package,
# the viewer and the chat app each built with warnings as errors and tested. Needs Xcode; scripts/check.sh
# covers the Rust side, the binding's tests included.
# Extra arguments are forwarded to both SwiftPM commands (for example,
# --disable-sandbox when an outer sandbox cannot host SwiftPM's sandbox).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 tools/viewer/Requests/sync-recorded-streams.py --check
python3 scripts/test-check-swift-tests.py
scripts/viewer.sh rust
status=0
test_pid=
interrupt_tests() {
  trap '' INT TERM
  if [[ -n "$test_pid" ]]; then
    kill -TERM "$test_pid" 2>/dev/null || true
    wait "$test_pid" || true
  fi
  exit "$1"
}
trap 'interrupt_tests 130' INT
trap 'interrupt_tests 143' TERM
for package in swift/OpenUINative tools/viewer tools/chat; do
  if ! swift build --package-path "$package" --build-tests -Xswiftc -warnings-as-errors "$@"; then
    status=1
    continue
  fi
  python3 scripts/check-swift-tests.py "$package" -Xswiftc -warnings-as-errors "$@" &
  test_pid=$!
  wait "$test_pid" || status=1
  test_pid=
done
exit "$status"
