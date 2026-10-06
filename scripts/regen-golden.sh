#!/usr/bin/env bash
# Regenerate conformance/golden/ from the vendored reference parser. Needs Bun.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/reference"
bun install --silent
OPENUI_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1 bun run.ts golden
