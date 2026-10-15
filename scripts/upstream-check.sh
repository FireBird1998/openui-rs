#!/usr/bin/env bash
# Has upstream moved? Lists the lang-core commits on upstream's main since the
# pinned one (reference/UPSTREAM). When there are any, it vendors the newest
# lang-core into <out-dir>, computes our corpus's goldens with it there, and
# lists which cases' outputs change. Never touches tracked files; moving the pin
# is scripts/update-reference.sh. A hunt runs this; it also runs alone.
#
#   scripts/upstream-check.sh <out-dir>
#
# Writes into <out-dir>: status (current, new, not-on-main or unreachable),
# source, main, commits.tsv, and with new commits newest, lang-core/,
# goldens (computed, or why not), golden/ (only the changed files) and
# changed.json, and packages/, what that lang-core imports. Upstream's history is cached, without file contents, in
# target/upstream.git. Without network it uses the cache if there is one;
# without Bun it lists the commits only.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/upstream.sh"
out="${1:?usage: scripts/upstream-check.sh <out-dir>}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
cache="$ROOT/target/upstream.git"
pin="$(sed -n 's/^commit: //p' "$ROOT/reference/UPSTREAM")"
PKG="packages/lang-core"

if [[ -d "$cache" ]]; then
  fetched="$(git -C "$cache" fetch -q origin 2>&1)" && reached=1 || reached=0
else
  mkdir -p "$ROOT/target"
  fetched="$(git clone -q --bare --filter=blob:none --single-branch --branch main "$UPSTREAM_URL" "$cache" 2>&1)" &&
    reached=1 || { reached=0; rm -rf "$cache"; }
fi
if [[ "$reached" == 1 ]]; then
  date +%Y-%m-%d > "$cache/fetched"
  echo "GitHub, just now" > "$out/source"
elif [[ -d "$cache" ]]; then
  echo "the copy fetched on $(cat "$cache/fetched"), because GitHub didn't answer" > "$out/source"
  echo "$fetched" >&2
else
  echo "unreachable" > "$out/status"
  echo "nothing: GitHub didn't answer and there's no cached copy" > "$out/source"
  echo "$fetched" >&2
  exit 0
fi

git -C "$cache" rev-parse main > "$out/main"
if ! git -C "$cache" merge-base --is-ancestor "$pin" main 2>/dev/null; then
  echo "not-on-main" > "$out/status"
  exit 0
fi
git -C "$cache" log --format='%H%x09%ad%x09%s' --date=short "$pin..main" -- "$PKG" > "$out/commits.tsv"
if [[ ! -s "$out/commits.tsv" ]]; then
  echo "current" > "$out/status"
  exit 0
fi
echo "new" > "$out/status"

newest="$(head -n 1 "$out/commits.tsv" | cut -f 1)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fetch_upstream "$newest" "$tmp" "" "$PKG" pnpm-workspace.yaml >/dev/null
if [[ ! -d "$tmp/$PKG/src" ]]; then
  echo "not computed: fetching lang-core at ${newest:0:12} failed" > "$out/goldens"
  exit 0
fi
rm -rf "$out/lang-core"
cp -R "$tmp/$PKG" "$out/lang-core"
version="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$out/lang-core/package.json")"
echo "$newest $version" > "$out/newest"

if ! command -v bun >/dev/null; then
  echo "not computed: Bun isn't installed" > "$out/goldens"
  exit 0
fi
# The packages that lang-core imports, installed beside it, not in reference/.
rm -rf "$out/packages" "$out/golden"
mkdir "$out/packages"
python3 -I "$ROOT/scripts/hunt.py" packages "$out/lang-core" "$tmp/pnpm-workspace.yaml" \
  "$ROOT/reference/package.json" > "$out/packages/package.json"
if ! (cd "$out/packages" && bun install --silent) > "$out/goldens.log" 2>&1; then
  echo "not computed: installing the packages it imports failed (see goldens.log)" > "$out/goldens"
  exit 0
fi
if "$ROOT/scripts/regen-golden.sh" --lang-core "$out/lang-core" --node-modules "$out/packages/node_modules" \
  --into "$out/golden" >> "$out/goldens.log" 2>&1; then
  python3 -I "$ROOT/scripts/hunt.py" compare-goldens "$ROOT/conformance/golden" "$out/golden" "$out/changed.json"
  echo "computed" > "$out/goldens"
else
  echo "not computed: the reference's scripts failed with the new lang-core (see goldens.log)" > "$out/goldens"
fi
