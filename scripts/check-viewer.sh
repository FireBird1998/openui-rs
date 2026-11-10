#!/usr/bin/env bash
# The viewer's check: its binding rebuilt, the app and its tests built with
# warnings as errors, and its Swift tests. Needs Xcode; scripts/check.sh
# covers the Rust side, the binding's tests included.
# Extra arguments are forwarded to both SwiftPM commands (for example,
# --disable-sandbox when an outer sandbox cannot host SwiftPM's sandbox).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 viewer/Requests/sync-recorded-streams.py --check
scripts/viewer.sh rust
swift build --package-path viewer --build-tests -Xswiftc -warnings-as-errors "$@"
swift test --package-path viewer -Xswiftc -warnings-as-errors "$@"
