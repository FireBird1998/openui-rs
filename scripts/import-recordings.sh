#!/usr/bin/env bash
# Import the viewer's recordings into the corpus: copies each
# *.recording.json from the folder into conformance/recordings/, turns every
# recording there into cases (reference/import-recordings.ts), and regenerates
# the goldens. Needs Bun.
#
#   scripts/import-recordings.sh [<folder>]
#
# The folder defaults to the viewer's own, in Application Support. With no
# new recordings it rebuilds the cases from those already in the corpus.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
folder="${1:-$HOME/Library/Application Support/dev.openui-rs.viewer/Recordings}"
mkdir -p "$ROOT/conformance/recordings"
copied=0
if [[ -d "$folder" ]]; then
  for file in "$folder"/*.recording.json; do
    [[ -e "$file" ]] || continue
    cp "$file" "$ROOT/conformance/recordings/"
    copied=$((copied + 1))
  done
fi
echo "Copied $copied recordings from $folder"
(cd "$ROOT/reference" && bun --no-install import-recordings.ts)
"$ROOT/scripts/regen-golden.sh"
