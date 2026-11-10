#!/usr/bin/env bash
# The nightly hunt: one scripts/hunt.sh run that nobody has to remember,
# started by cron on a Mac that can spare the time (the worker Mac, not the
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
# The job is one line in the user's crontab, at hour H (2 by default) for N
# minutes (120 by default): cron runs it whether or not anyone is logged in at
# the screen, which a launchd agent needs. Installing it is a standing
# configuration on that Mac: the maintainer's decision, each time.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

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

# The job's line in the user's crontab, found again by its marker.
MARK="# openui-rs nightly hunt"
others() { crontab -l 2> /dev/null | grep -vF "$MARK"; }
case "$action" in
  uninstall)
    others | crontab -
    echo "Removed the nightly hunt from $(id -un)'s crontab"
    exit 0
    ;;
  install)
    { others; echo "0 $hour * * * /bin/bash $ROOT/scripts/nightly-hunt.sh --minutes $minutes $MARK"; } | crontab - || exit 1
    echo "Installed: a $minutes-minute hunt at $hour:00 every night, in $ROOT ($(id -un)'s crontab)"
    exit 0
    ;;
esac

# cron gives a bare PATH: cargo, Bun and Homebrew's tools are added here.
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
