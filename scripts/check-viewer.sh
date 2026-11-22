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
for package in swift/OpenUINative tools/viewer tools/chat; do
  if ! swift build --package-path "$package" --build-tests -Xswiftc -warnings-as-errors "$@"; then
    status=1
    continue
  fi
  python3 scripts/check-swift-tests.py "$package" -Xswiftc -warnings-as-errors "$@" || status=1
done
exit "$status"
