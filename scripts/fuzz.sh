#!/usr/bin/env bash
# Differential fuzzing against the reference parser. Needs Bun.
#
#   scripts/fuzz.sh [--seed N] [--count N] [--from K] [--only K] [--seconds S]
#       [--findings DIR] [--no-save] [--edits | --invariants | --runtime]
#   scripts/fuzz.sh --runtime --replay FILE --only 0
#   scripts/fuzz.sh --runtime-case --seed N --only K  # print a v2 replay
# Runtime mode always saves reviewable findings, never changes the corpus.
#
# Shrunk mismatches are saved to conformance/fuzz.json as corpus cases (with
# --edits, merge and serialize ones to merge-fuzz.json and serialize-fuzz.json),
# and the goldens are regenerated so the normal test run covers them without Bun.
# --invariants checks the rules in examples/fuzz/invariants.rs on both parsers
# and reports what breaks them; it saves nothing.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[[ -d reference/node_modules ]] || { echo 'fuzz.sh: cached reference/node_modules required; nothing installed' >&2; exit 1; }
export CARGO_NET_OFFLINE=true OPENUI_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1
saved() { cat conformance/fuzz.json conformance/merge-fuzz.json conformance/serialize-fuzz.json 2>/dev/null || true; }
before="$(saved)"
cargo run -q --release -p openui-lang --example fuzz -- "$@"
if [[ "$(saved)" != "$before" ]]; then
  "$ROOT/scripts/regen-golden.sh"
  echo "New fuzz cases saved; cargo test now checks them."
fi
