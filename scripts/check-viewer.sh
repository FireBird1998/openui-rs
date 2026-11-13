#!/usr/bin/env bash
# The Swift side's check: the binding rebuilt, then the OpenUINative package
# and the viewer each built with warnings as errors and tested. Needs Xcode; scripts/check.sh
# covers the Rust side, the binding's tests included.
# Extra arguments are forwarded to both SwiftPM commands (for example,
# --disable-sandbox when an outer sandbox cannot host SwiftPM's sandbox).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 tools/viewer/Requests/sync-recorded-streams.py --check
scripts/viewer.sh rust
for package in swift/OpenUINative tools/viewer; do
  swift build --package-path "$package" --build-tests -Xswiftc -warnings-as-errors "$@"
  swift test --package-path "$package" -Xswiftc -warnings-as-errors "$@"
done
