#!/usr/bin/env bash
# The viewer's timing run: an optimized build plays a fixed set of programs in
# the real window and reports, per update, the median and worst time to parse,
# cross into Swift, decode and redraw, plus the main thread's longest stall.
# Needs Xcode and a logged-in screen; takes about three minutes.
#
#   scripts/viewer-timing.sh
#
# Two ways of playing each program:
#   steady  30 chunks a second, so most updates carry one or two chunks: what
#           streaming from a model looks like.
#   stress  one code point per chunk as fast as possible, so each update
#           carries hundreds of chunks: what jumping around looks like.
set -euo pipefail
cd "$(dirname "$0")/.."
scripts/viewer.sh build --release >/dev/null
app=target/viewer/release/Viewer.app/Contents/MacOS/Viewer

run() { # program chunk speed seconds
  VIEWER_PLAY="$1" VIEWER_CHUNK="$2" VIEWER_SPEED="$3" VIEWER_SECONDS="$4" \
    perl -e 'alarm 200; exec @ARGV' "$app" 2>&1 \
    | grep -v -e 'NSTableView delegate' -e 'Charts: Custom UnitPoint' || true
}

echo "== steady: 30 chunks/s"
run bench/dashboard 16 30 16
run gallery 16 30 12
run synthetic/dashboard-x2 16 30 30
run synthetic/dashboard-x8 64 30 20
echo
echo "== stress: 1 code point per chunk, as fast as possible"
for program in gallery bench/dashboard synthetic/dashboard-x2 synthetic/dashboard-x8; do
  run "$program" 1 10000 6
done
