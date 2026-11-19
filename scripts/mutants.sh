#!/usr/bin/env bash
# Mutation testing (cargo-mutants): plants a bug at a time in a group of the
# core's modules and runs the offline suite against each, with live diffs off,
# to find code the suite wouldn't notice breaking. Run it on AC power: a mutant
# costs a rebuild plus the suite.
#
#   scripts/mutants.sh parsing|rest|runtime|query [--jobs N] [cargo-mutants options...]
#
#   parsing  the lexer, statements, expressions and the AST, the preprocessor,
#            the stall guard,
#            the parser, materializing and validation, evaluating state
#            defaults, the streaming table and parser, and results (testing
#            ticket 04)
#   rest     merging, converting back to source, prompts, error hints, JSON,
#            the library, builtins, the crate's front page, the CLI, and the
#            binding crate (testing ticket 05)
#   runtime  all openui-runtime source modules, including nested tree modules
#            (#40) and the query manager; the maintainer starts the full run
#   query    the query manager alone (src/query.rs and src/query/, #95), tested
#            by the runtime's suite and the session's, which replays the
#            renderer's query and mutation effects against upstream
#
# Results go to target/mutants/<group>/mutants.out/ (missed.txt, caught.txt,
# timeout.txt, outcomes.json). Mutants that change nothing are listed, with
# reasons, in .cargo/mutants.toml.
#
# A mutant that stops the parser advancing can make a test, or the CLI a test
# starts, allocate without bound, faster than cargo-mutants' timeout ends it.
# So scripts/mutants-watchdog.sh runs beside cargo-mutants and kills any
# process from a mutant's build past MUTANTS_MEMORY_LIMIT_MB (default 2048):
# the mutant counts as caught, and the kill is logged.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
group="${1:-}"
shift || true
case "$group" in
  parsing)
    files=(lexer statements expr ast preprocess parser materialize validation evaluate table stream result progress)
    packages=(-p openui-lang) ;;
  rest)
    files=(merge serialize prompt enrich js library builtins lib bin/openui-lang)
    packages=(-p openui-lang -p openui-ffi) ;;
  runtime)
    files=()
    packages=(-p openui-runtime) ;;
  query)
    files=()
    packages=(-p openui-runtime --test-package openui-runtime --test-package openui-session) ;;
  *) echo "usage: scripts/mutants.sh parsing|rest|runtime|query [--jobs N] [cargo-mutants options...]" >&2; exit 2 ;;
esac
args=()
if [[ "$group" == runtime ]]; then
  args=(--file "crates/openui-runtime/src/**/*.rs" --file "crates/openui-runtime/src/*.rs")
elif [[ "$group" == query ]]; then
  args=(--file "crates/openui-runtime/src/query.rs" --file "crates/openui-runtime/src/query/*.rs")
else
  for f in "${files[@]}"; do args+=(--file "crates/openui-lang/src/$f.rs"); done
fi
[[ "$group" != rest ]] || args+=(--file "crates/openui-ffi/src/*.rs")
mkdir -p "target/mutants/$group"

case " $* " in
  *' --list '*|*' --list-files '*) ;; # No test processes to watch.
  *)
    "$ROOT/scripts/mutants-watchdog.sh" $$ &
    watchdog_pid=$!
    trap 'kill "$watchdog_pid" 2>/dev/null || true' EXIT ;;
esac

OPENUI_NO_LIVE_DIFFS=1 CARGO_NET_OFFLINE=true cargo mutants "${packages[@]}" "${args[@]}" --output "target/mutants/$group" "$@"
