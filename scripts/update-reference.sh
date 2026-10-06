#!/usr/bin/env bash
# Vendor the reference parser (@openuidev/lang-core source) at an upstream commit.
#
#   scripts/update-reference.sh <commit> [<local thesysdev/openui checkout>]
#
# With a local checkout any commit name works; from GitHub it must be the full
# 40-character hash. Afterwards run scripts/import-corpus.sh and
# scripts/regen-golden.sh.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/upstream.sh"
PKG="packages/lang-core"
DEST="$ROOT/reference/lang-core"

commit="${1:?usage: update-reference.sh <commit> [<local openui checkout>]}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
full="$(fetch_upstream "$commit" "$tmp" "${2:-}" "$PKG" LICENSE)"

rm -rf "$DEST"
mkdir -p "$DEST"
cp -R "$tmp/$PKG/src" "$DEST/src"
# Upstream's MIT notice travels with its code.
cp "$tmp/LICENSE" "$DEST/LICENSE"
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
