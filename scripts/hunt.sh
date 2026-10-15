#!/usr/bin/env bash
# A hunt: searches for findings for a set time, then leaves a report and the
# shrunk cases in findings/<date>-seed<seed>/, which git ignores. It runs
# differential fuzzing with a fresh seed, for parsing and then for edits, with
# the upstream check (scripts/upstream-check.sh) alongside. A hunt never commits
# and never changes tracked files: it compares them before and after, and fails
# if they changed. Fuzzing needs Bun; the first hunt also builds the fuzzer.
#
#   scripts/hunt.sh --minutes N [--seed S] [--no-upstream]
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

usage() {
  echo "usage: scripts/hunt.sh --minutes N [--seed S] [--no-upstream]" >&2
  exit 2
}
minutes=""
seed=""
upstream=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --minutes) [[ $# -ge 2 ]] || usage; minutes="$2"; shift 2 ;;
    --seed) [[ $# -ge 2 ]] || usage; seed="$2"; shift 2 ;;
    --no-upstream) upstream=0; shift ;;
    *) usage ;;
  esac
done
[[ "$minutes" =~ ^[1-9][0-9]*$ ]] || usage
[[ -z "$seed" || "$seed" =~ ^[0-9]+$ ]] || usage

started="$(date +%s)"
deadline=$((started + minutes * 60 - 5)) # 5 s left over for the report
seed="${seed:-$(od -An -N4 -tu4 /dev/urandom | tr -d ' ')}"
slow_ms=1000
hunt="findings/$(date +%Y-%m-%d-%H%M%S)-seed$seed"
mkdir -p "$hunt"

# Every tracked file's content, and the untracked files git doesn't ignore.
snapshot() {
  git ls-files -z --cached --others --exclude-standard | xargs -0 shasum 2>&1
}
snapshot > "$hunt/.tracked-before"

dirty=false
[[ -z "$(git status --porcelain)" ]] || dirty=true
cat > "$hunt/meta.json" <<META
{"started": "$(date '+%Y-%m-%d %H:%M')", "minutes": $minutes, "seed": $seed, "commit": "$(git rev-parse HEAD)", "dirty": $dirty, "pin": "$(sed -n 's/^commit: //p' reference/UPSTREAM)", "version": "$(sed -n 's/^package: @openuidev\/lang-core //p' reference/UPSTREAM)", "slow_ms": $slow_ms}
META
echo "Hunting for $minutes minutes with seed $seed, into $hunt/"

# Each background job gets its own process group, so stopping one stops what
# it started too (the fuzzer's Bun process, the upstream check's git and Bun).
set -m

# Waits for a background job until it finishes or the time `until` passes, then
# stops its process group. Writes ok, exit N or killed to the status file.
watch() {
  local pid="$1" until="$2" status_file="$3" result=""
  while kill -0 "$pid" 2>/dev/null; do
    if (($(date +%s) >= until)); then
      kill -TERM -- "-$pid" 2>/dev/null
      sleep 2
      kill -KILL -- "-$pid" 2>/dev/null
      result="killed"
      break
    fi
    sleep 1
  done
  wait "$pid" 2>/dev/null
  local code=$?
  if [[ -z "$result" ]]; then
    if [[ "$code" == 0 ]]; then result="ok"; else result="exit $code"; fi
  fi
  echo "$result" > "$status_file"
}

if [[ "$upstream" == 1 ]]; then
  scripts/upstream-check.sh "$hunt/upstream" > "$hunt/upstream.log" 2>&1 &
  upstream_pid=$!
else
  echo "skipped: --no-upstream" > "$hunt/status-upstream"
fi

skip=""
if ! command -v bun >/dev/null; then
  skip="Bun isn't installed"
elif ! cargo build -q --locked --release -p openui-lang --example fuzz > "$hunt/build.log" 2>&1; then
  skip="the fuzzer didn't build (see build.log)"
elif ! (cd reference && bun install --frozen-lockfile --silent) >> "$hunt/build.log" 2>&1; then
  skip="the reference's packages didn't install (see build.log)"
fi
if [[ -n "$skip" ]]; then
  echo "skipped: $skip" > "$hunt/status-parse"
  echo "skipped: $skip" > "$hunt/status-edits"
  echo "Fuzzing skipped: $skip"
else
  # Parsing gets the first half of the time left and edits the second. Each
  # fuzzer stops itself before its share ends; one still running at the end is
  # stopped, and its last batch reported as a probable hang.
  now="$(date +%s)"
  parse_until=$((now + (deadline - now) / 2))
  for check in parse edits; do
    now="$(date +%s)"
    until=$parse_until
    [[ "$check" == parse ]] || until=$deadline
    share=$((until - now))
    grace=$((share / 10 < 15 ? share / 10 : 15))
    flags="--seed $seed --seconds $((share - grace)) --slow-ms $slow_ms --findings $hunt"
    [[ "$check" == parse ]] || flags="--edits $flags"
    echo "Fuzzing $check for $((share - grace)) s"
    # shellcheck disable=SC2086 # the flags are words
    target/release/examples/fuzz $flags > "$hunt/$check.log" 2>&1 &
    watch $! "$until" "$hunt/status-$check" 2>/dev/null # not bash's notice of how it ended
  done
fi

if [[ "$upstream" == 1 ]]; then
  echo "Finishing the upstream check"
  watch "$upstream_pid" "$deadline" "$hunt/status-upstream" 2>/dev/null
fi
set +m

snapshot > "$hunt/.tracked-after"
diff "$hunt/.tracked-before" "$hunt/.tracked-after" | grep '^[<>]' |
  sed -E 's/^[<>] ([0-9a-f]{40}  )?//' | sort -u > "$hunt/tracked-changes.txt"

python3 -I scripts/hunt.py report "$hunt" $(($(date +%s) - started)) || exit 1
echo
sed -n '/^## Summary/,/^## /p' "$hunt/report.md" | sed '1d;$d'
echo "The report: $hunt/report.md"
if [[ -s "$hunt/tracked-changes.txt" ]]; then
  echo "Tracked files changed during the hunt:" >&2
  cat "$hunt/tracked-changes.txt" >&2
  exit 1
fi
