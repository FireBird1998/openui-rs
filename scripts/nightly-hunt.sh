#!/usr/bin/env bash
# The nightly hunt: one scripts/hunt.sh run that nobody has to remember,
# started by launchd on a Mac that can spare the time (the worker Mac, not the
# one the maintainer works on). It runs only on AC power, brings a clean
# checkout up to date first, and never commits: like every hunt it leaves its
# report and shrunk cases in findings/, which git ignores. The latest run's
# folder and exit status are in findings/nightly-latest.txt, and everything it
# printed in findings/nightly.log.
#
#   scripts/nightly-hunt.sh [--minutes N]                      run one now
#   scripts/nightly-hunt.sh --install [--hour H] [--minutes N] install the job
#   scripts/nightly-hunt.sh --uninstall                        remove it
#
# The job is a launchd agent of the logged-in user (dev.openui-rs.nightly-hunt),
# at hour H (2 by default) for N minutes (120 by default). Installing it is a
# standing configuration on that Mac: the maintainer's decision, each time.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LABEL="dev.openui-rs.nightly-hunt"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

usage() {
  echo "usage: scripts/nightly-hunt.sh [--install [--hour H] | --uninstall] [--minutes N]" >&2
  exit 2
}
action=run
minutes=120
hour=2
while [[ $# -gt 0 ]]; do
  case "$1" in
    --install) action=install; shift ;;
    --uninstall) action=uninstall; shift ;;
    --minutes) [[ $# -ge 2 ]] || usage; minutes="$2"; shift 2 ;;
    --hour) [[ $# -ge 2 ]] || usage; hour="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[[ "$minutes" =~ ^[1-9][0-9]*$ ]] || usage
[[ "$hour" =~ ^([0-9]|1[0-9]|2[0-3])$ ]] || usage

case "$action" in
  uninstall)
    launchctl bootout "gui/$(id -u)/$LABEL" 2> /dev/null
    rm -f "$PLIST"
    echo "Removed $LABEL"
    exit 0
    ;;
  install)
    mkdir -p "$(dirname "$PLIST")"
    cat > "$PLIST" <<PLIST_END
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/bash</string>
    <string>$ROOT/scripts/nightly-hunt.sh</string>
    <string>--minutes</string>
    <string>$minutes</string>
  </array>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>$hour</integer><key>Minute</key><integer>0</integer></dict>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
PLIST_END
    launchctl bootout "gui/$(id -u)/$LABEL" 2> /dev/null
    launchctl bootstrap "gui/$(id -u)" "$PLIST" || exit 1
    echo "Installed $LABEL: a $minutes-minute hunt at $hour:00 every night, in $ROOT"
    exit 0
    ;;
esac

# launchd gives a bare PATH: cargo, Bun and Homebrew's tools are added here.
export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
cd "$ROOT"
mkdir -p findings
exec >> findings/nightly.log 2>&1
echo "== $(date '+%Y-%m-%d %H:%M') nightly hunt, $minutes minutes"

if ! pmset -g batt | grep -q "AC Power"; then
  echo "On battery: no hunt tonight."
  exit 0
fi
# One at a time: a hunt still running from the night before keeps the Mac.
if ! mkdir findings/.nightly-lock 2> /dev/null; then
  echo "Another nightly hunt holds findings/.nightly-lock: no hunt tonight."
  exit 0
fi
trap 'rmdir findings/.nightly-lock' EXIT

# Hunt on the newest main when nothing here is in the way of taking it.
if [[ -z "$(git status --porcelain)" ]]; then
  git pull --ff-only -q || echo "Couldn't pull; hunting on $(git rev-parse --short HEAD)."
else
  echo "The checkout has changes; hunting on it as it is."
fi

before="$(ls -d findings/*-seed* 2> /dev/null | sort | tail -1)"
scripts/hunt.sh --minutes "$minutes"
status=$?
latest="$(ls -d findings/*-seed* 2> /dev/null | sort | tail -1)"
[[ "$latest" != "$before" ]] || latest="(none)"
printf '%s\nexit %s\ncommit %s\n' "$latest" "$status" "$(git rev-parse --short HEAD)" > findings/nightly-latest.txt
echo "== $(date '+%Y-%m-%d %H:%M') done: $latest, exit $status"
exit "$status"
