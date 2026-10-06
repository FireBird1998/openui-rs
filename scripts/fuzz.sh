#!/usr/bin/env bash
# Differential fuzzing against the reference parser. Needs Bun.
#
#   scripts/fuzz.sh [--seed N] [--count N] [--only K] [--no-save] [--edits]
#
# Shrunk mismatches are saved to conformance/fuzz.json as corpus cases (with
# --edits, merge and serialize ones to merge-fuzz.json and serialize-fuzz.json),
# and the goldens are regenerated so the normal test run covers them without Bun.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
(cd reference && bun install --silent)
saved() { cat conformance/fuzz.json conformance/merge-fuzz.json conformance/serialize-fuzz.json 2>/dev/null || true; }
before="$(saved)"
cargo run -q --release -p openui-lang --example fuzz -- "$@"
if [[ "$(saved)" != "$before" ]]; then
  "$ROOT/scripts/regen-golden.sh"
  echo "New fuzz cases saved; cargo test now checks them."
fi
