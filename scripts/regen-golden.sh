#!/usr/bin/env bash
# Regenerate conformance/golden/ from the vendored reference parser. Needs Bun.
#
#   scripts/regen-golden.sh [--lang-core <dir>] [--node-modules <dir>] [--into <dir>]
#
# The reference's scripts run on a scratch copy of the corpus, and their goldens
# replace conformance/golden/ only once all of them succeed, so a crash in the
# reference leaves the old goldens in place. --lang-core runs another copy of
# lang-core (a folder holding its src/) instead of reference/lang-core, with
# the packages in --node-modules if it needs others, and --into writes the
# goldens to a new folder instead of replacing conformance/golden/. The
# upstream check (scripts/upstream-check.sh) uses all three.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
lang_core="$ROOT/reference/lang-core"
node_modules="$ROOT/reference/node_modules"
into=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --lang-core) lang_core="$(cd "$2" && pwd)"; shift 2 ;;
    --node-modules) node_modules="$(cd "$2" && pwd)"; shift 2 ;;
    --into) into="$2"; shift 2 ;;
    *) echo "usage: scripts/regen-golden.sh [--lang-core <dir>] [--node-modules <dir>] [--into <dir>]" >&2; exit 2 ;;
  esac
done
if [[ -n "$into" && -e "$into" ]]; then
  echo "regen-golden.sh: $into already exists" >&2
  exit 2
fi

# Dependencies must already exist. Reference runners never install packages.
if [[ ! -d "$node_modules" ]]; then
  echo "regen-golden.sh: missing cached node_modules: $node_modules" >&2
  exit 1
fi
mkdir -p "$ROOT/target"
work="$(mktemp -d "$ROOT/target/golden.XXXXXX")"
trap 'rm -rf "$work"' EXIT
mkdir "$work/reference" "$work/conformance"
cp "$ROOT"/reference/*.ts "$ROOT/reference/package.json" "$work/reference/"
ln -s "$node_modules" "$work/reference/node_modules"
cp -R "$lang_core" "$work/reference/lang-core"
for entry in "$ROOT"/conformance/*; do
  [[ "$(basename "$entry")" == golden ]] || cp -R "$entry" "$work/conformance/"
done

(
  cd "$work/reference"
  export OPENUI_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1
  unset OPENUI_RUNTIME_TELEMETRY_ENABLED
  bun --no-install run.ts golden
  bun --no-install messages.ts
  bun --no-install prompts.ts golden
  bun --no-install edits.ts golden
  bun --no-install runtime.ts golden
  bun --no-install runtime.ts fuzz-golden
  bun --no-install renderer-action-lookups.ts
  bun --no-install renderer-form-prototypes.ts
  bun --no-install tree-interface.ts > ../conformance/golden/runtime-interface.json
  bun --no-install action-definitions.ts
  bun --no-install function-definitions.ts
  bun --no-install function-results.ts
  bun --no-install session-data.ts
  bun --no-install mcp-results.ts
  bun --no-install parser-prototype-assignments.ts
  bun --no-install core-divergences.ts
  bun --no-install query-default-cycles.ts
  bun --no-install function-undefined-arguments.ts
  bun --no-install renderer-reset-native-defaults.ts
)
if [[ -n "$into" ]]; then
  mkdir -p "$(dirname "$into")"
  mv "$work/conformance/golden" "$into"
else
  mv "$ROOT/conformance/golden" "$work/golden.old"
  mv "$work/conformance/golden" "$ROOT/conformance/golden"
fi
