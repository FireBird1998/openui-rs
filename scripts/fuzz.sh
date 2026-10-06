#!/usr/bin/env bash
# Differential fuzzing against the reference parser. Needs Bun.
#
#   scripts/fuzz.sh [--seed N] [--count N] [--only K] [--no-save]
#
# Shrunk mismatches are saved to conformance/fuzz.json as corpus cases, and the
# goldens are regenerated so the normal test run covers them without Bun.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
(cd reference && bun install --silent)
before="$(cat conformance/fuzz.json 2>/dev/null || true)"
cargo run -q --release -p openui-lang --example fuzz -- "$@"
if [[ "$(cat conformance/fuzz.json 2>/dev/null || true)" != "$before" ]]; then
  "$ROOT/scripts/regen-golden.sh"
  echo "New fuzz cases saved; cargo test now checks them."
fi
