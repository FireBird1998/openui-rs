#!/usr/bin/env bash
# Mutation testing (cargo-mutants): plants a bug at a time in a group of the
# core's modules and runs the offline suite against each, with live diffs off,
# to find code the suite wouldn't notice breaking. Run it on AC power: a mutant
# costs a rebuild plus the suite.
#
#   scripts/mutants.sh parsing|rest [--jobs N] [cargo-mutants options...]
#
#   parsing  the lexer, statements, expressions and the AST, the preprocessor,
#            the parser, materializing and validation, evaluating state
#            defaults, the streaming table and parser, and results (testing
#            ticket 04)
#   rest     merging, converting back to source, prompts, error hints, JSON,
#            the library, builtins, the crate's front page, the CLI, and the
#            binding crate (testing ticket 05)
#
# Results go to target/mutants/<group>/mutants.out/ (missed.txt, caught.txt,
# timeout.txt, outcomes.json). Mutants that change nothing are listed, with
# reasons, in .cargo/mutants.toml.
#
# A mutant that stops the parser advancing can make a test allocate without
# bound, faster than cargo-mutants' timeout ends it, and macOS has no
# per-process memory limit: on 2026-11-06 two such tests reached 140 GB and
# 70 GB and the machine panicked. So a watchdog kills any test process from a
# mutant's build that holds more than MUTANTS_MEMORY_LIMIT_MB (default 2048;
# the suite peaks near 600 MB), and on macOS, when free memory falls under
# 20%, the largest one over 1 GB. The test fails, so the mutant counts as
# caught, and the kill is logged.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
group="${1:-}"
shift || true
case "$group" in
  parsing)
    files=(lexer statements expr ast preprocess parser materialize validation evaluate table stream result)
    packages=(-p openui-lang) ;;
  rest)
    files=(merge serialize prompt enrich js library builtins lib bin/openui-lang)
    packages=(-p openui-lang -p openui-ffi) ;;
  *) echo "usage: scripts/mutants.sh parsing|rest [--jobs N] [cargo-mutants options...]" >&2; exit 2 ;;
esac
args=()
for f in "${files[@]}"; do args+=(--file "crates/openui-lang/src/$f.rs"); done
[[ "$group" != rest ]] || args+=(--file "crates/openui-ffi/src/*.rs")
mkdir -p "target/mutants/$group"

limit_kb=$(( ${MUTANTS_MEMORY_LIMIT_MB:-2048} * 1024 ))
# The PIDs to kill, with their size: a test binary runs from
# <build dir>/target/<profile>/deps/, and cargo-mutants' build dirs are named
# cargo-mutants-<crate>-<random>.tmp; rustc's own path never matches.
runaways() {
  local free=100
  [[ "$(uname)" != Darwin ]] || free="$(sysctl -n kern.memorystatus_level 2>/dev/null || echo 100)"
  ps -eo pid=,rss=,args= | awk -v limit="$limit_kb" -v free="$free" '
    $3 ~ /cargo-mutants-[^ \/]*\.tmp\/target\/[^ ]*\/deps\// {
      if ($2 > limit) { print $1, $2, $3; next }
      if ($2 > big) { big = $2; line = $1 " " $2 " " $3 }
    }
    END { if (free < 20 && big > 1048576) print line }'
}
watchdog() {
  local parent=$$ pid kb command
  while kill -0 "$parent" 2>/dev/null; do
    while read -r pid kb command; do
      kill -9 "$pid" 2>/dev/null &&
        echo "mutants.sh: killed a test holding $(( kb / 1024 )) MB: $command" >&2
    done < <(runaways || true)
    sleep 0.25
  done
}
watchdog &
watchdog_pid=$!
trap 'kill "$watchdog_pid" 2>/dev/null || true' EXIT

OPENUI_NO_LIVE_DIFFS=1 cargo mutants "${packages[@]}" "${args[@]}" --output "target/mutants/$group" "$@"
