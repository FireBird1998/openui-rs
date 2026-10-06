#!/usr/bin/env bash
# Vendor the reference parser (@openuidev/lang-core source) at an upstream commit.
#
#   scripts/update-reference.sh <commit> [<local thesysdev/openui checkout>]
#
# With a local checkout the files come from `git archive` there, and any commit
# name works. Otherwise they're fetched from GitHub, which needs the full
# 40-character commit hash (or a branch or tag name). Afterwards regenerate the goldens with
# scripts/regen-golden.sh.
set -euo pipefail

UPSTREAM_URL="https://github.com/thesysdev/openui.git"
PKG="packages/lang-core"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/reference/lang-core"

commit="${1:?usage: update-reference.sh <commit> [<local openui checkout>]}"
local_repo="${2:-}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if [[ -n "$local_repo" ]]; then
  git -C "$local_repo" archive "$commit" "$PKG" | tar -x -C "$tmp"
  full="$(git -C "$local_repo" rev-parse "$commit")"
else
  if [[ "$commit" =~ ^[0-9a-f]{7,39}$ ]]; then
    echo "GitHub won't fetch a shortened hash; pass the full 40-character commit." >&2
    exit 1
  fi
  git -C "$tmp" init -q
  git -C "$tmp" remote add origin "$UPSTREAM_URL"
  git -C "$tmp" sparse-checkout set "$PKG"
  git -C "$tmp" fetch -q --depth 1 origin "$commit"
  git -C "$tmp" checkout -q FETCH_HEAD
  full="$(git -C "$tmp" rev-parse HEAD)"
fi

rm -rf "$DEST"
mkdir -p "$DEST"
cp -R "$tmp/$PKG/src" "$DEST/src"
cp "$tmp/$PKG/package.json" "$DEST/package.json.upstream"
version="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$DEST/package.json.upstream")"

cat > "$ROOT/reference/UPSTREAM" <<META
repository: $UPSTREAM_URL
commit: $full
package: @openuidev/lang-core $version
path: $PKG/src
vendored: $(date +%Y-%m-%d)
META
echo "Vendored @openuidev/lang-core $version at ${full:0:12} into reference/lang-core"
