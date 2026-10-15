#!/usr/bin/env bash
# The viewer's check: its binding rebuilt, the app and its tests built with
# warnings as errors, and its Swift tests. Needs Xcode; scripts/check.sh
# covers the Rust side, the binding's tests included.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/viewer.sh rust
swift build --package-path viewer --build-tests -Xswiftc -warnings-as-errors
swift test --package-path viewer -Xswiftc -warnings-as-errors
