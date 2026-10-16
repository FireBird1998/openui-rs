#!/usr/bin/env bash
# Rebuild conformance/imported/ from upstream at the pinned commit: programs the
# reference test suites parse, the spec's conformance fixtures, the benchmark
# samples, examples from the docs and examples folders, and react-ui's
# component library with its prompt spec. Then regenerates the goldens. Needs
# Bun.
#
#   scripts/import-corpus.sh [<local thesysdev/openui checkout>]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/upstream.sh"
commit="$(sed -n 's/^commit: //p' "$ROOT/reference/UPSTREAM")"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fetch_upstream "$commit" "$tmp" "${1:-}" benchmarks docs/content examples spec packages/react-ui/src LICENSE >/dev/null

cd "$ROOT/reference"
bun install --silent
OPENUI_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1 bun import-corpus.ts "$tmp"
# The imported programs are upstream's, under its MIT notice.
cp "$tmp/LICENSE" "$ROOT/conformance/imported/LICENSE"
"$ROOT/scripts/regen-golden.sh"
