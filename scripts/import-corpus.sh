#!/usr/bin/env bash
# Rebuild conformance/imported/ from upstream at the pinned commit: programs the
# reference test suites parse, the benchmark samples, and examples from the
# docs and examples folders. Then regenerates the goldens. Needs Bun.
#
#   scripts/import-corpus.sh [<local thesysdev/openui checkout>]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/upstream.sh"
commit="$(sed -n 's/^commit: //p' "$ROOT/reference/UPSTREAM")"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fetch_upstream "$commit" "$tmp" "${1:-}" benchmarks docs/content examples >/dev/null

cd "$ROOT/reference"
bun install --silent
OPENUI_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1 bun import-corpus.ts "$tmp"
"$ROOT/scripts/regen-golden.sh"
