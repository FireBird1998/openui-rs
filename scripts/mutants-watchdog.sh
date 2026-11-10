#!/usr/bin/env bash
# Kills runaway processes from cargo-mutants' builds. scripts/mutants.sh starts
# it with its own PID; it stops once that process is gone.
#
#   scripts/mutants-watchdog.sh <pid to outlive>
#
# A mutant that stops the parser advancing can make a test, or the CLI a test
# starts, allocate without bound, faster than cargo-mutants' timeout ends it,
# and macOS has no per-process memory limit. On 2026-11-06 such processes
# reached 140 GB and 70 GB, and the worker Mac panicked; later two CLI
# processes this missed reached 85 GB and 74 GB, and it froze.
#
# So this kills any process run from a mutant's build whose memory passes
# MUTANTS_MEMORY_LIMIT_MB (default 2048; the suite peaks near 600 MB), and on
# macOS, when free memory falls under 20%, the largest one over 1 GB. The test
# fails, so the mutant counts as caught, and each kill is logged to stderr.
#
# On macOS a process's memory is its physical footprint, which counts
# compressed memory: under pressure the compressor keeps a runaway's RSS small
# (the 85 GB one showed 387 MB). Elsewhere it's RSS.
set -uo pipefail
parent="${1:?usage: scripts/mutants-watchdog.sh <pid to outlive>}"
limit_kb=$(( ${MUTANTS_MEMORY_LIMIT_MB:-2048} * 1024 ))

# "pid kb path" for each process to kill. Watched are the processes run from a
# mutant's build: the test binaries in <build>/target/<profile>/deps/ and the
# CLI in <build>/target/<profile>/. cargo-mutants names its builds
# cargo-mutants-<crate>-<random>.tmp; rustc's own path never matches.
runaways() {
  local free=100 procs sizes=""
  procs="$(ps -eo pid=,rss=,args= | awk '$3 ~ /cargo-mutants-[^ \/]*\.tmp\/target\// { print $1, $2, $3 }')"
  [[ -n "$procs" ]] || return 0
  if [[ "$(uname)" == Darwin ]]; then
    free="$(sysctl -n kern.memorystatus_level 2>/dev/null || echo 100)"
    # One line per process: "<name> [<pid>]: 64-bit    Footprint: <bytes> B …".
    # A process that has exited is left out.
    # shellcheck disable=SC2046
    sizes="$(footprint --noCategories -f bytes $(awk '{ print $1 }' <<<"$procs") 2>/dev/null |
      sed -nE 's/.*\[([0-9]+)\]: [0-9]+-bit +Footprint: ([0-9]+) B.*/\1 \2/p')"
  fi
  printf '%s\n--\n%s\n' "$sizes" "$procs" | awk -v limit="$limit_kb" -v free="$free" '
    $0 == "--" { listing = 1; next }
    !listing { if (NF == 2) size[$1] = int($2 / 1024); next }
    { kb = ($1 in size) ? size[$1] : $2 }
    kb > limit { print $1, kb, $3; next }
    kb > big { big = kb; line = $1 " " kb " " $3 }
    END { if (free < 20 && big > 1048576) print line }'
}

# Read cargo-mutants' own trace: its worker span associates the build directory
# with a mutant name. Match the basename because macOS logs /var while ps may
# report /private/var. Never edit cargo-mutants' records.
mutant_name() {
  local build="$1" log name="" candidate
  for log in "${MUTANTS_OUTPUT_ROOT:-target/mutants}"/*/mutants.out/debug.log; do
    [[ -r "$log" ]] || continue
    candidate="$(awk -v build="${build##*/}" '
      index($0, build "\"") && /:mutant\{name="/ {
        line = $0
        sub(/^.*:mutant\{name="/, "", line)
        sub(/"\}.*/, "", line)
        name = line
      }
      END { print name }' "$log")"
    [[ -z "$candidate" ]] || name="$candidate"
  done
  printf '%s' "${name:-description unavailable}"
}

while kill -0 "$parent" 2>/dev/null; do
  while read -r pid kb path; do
    # A test may restrict the watchdog to its own fake build directory.
    [[ -z "${MUTANTS_WATCHDOG_BUILD_ROOT:-}" || "$path" == "$MUTANTS_WATCHDOG_BUILD_ROOT/"* ]] || continue
    build="${path%%/target/*}"
    if kill -9 "$pid" 2>/dev/null; then
      description="$(mutant_name "$build")"
      echo "mutants-watchdog: $(date -u +%Y-%m-%dT%H:%M:%SZ) killed a process holding $(( kb / 1024 )) MB: $path; build: $build; mutant: $description" >&2
    fi
  done < <(runaways || true)
  sleep 0.25
done
