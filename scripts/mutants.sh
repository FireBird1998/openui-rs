#!/usr/bin/env bash
# Mutation testing (cargo-mutants): plants a bug at a time in a group of the
# core's modules and runs the offline suite against each, with live diffs off,
# to find code the suite wouldn't notice breaking. Run it on AC power: a mutant
# costs a rebuild plus the suite.
#
#   scripts/mutants.sh parsing|rest [--jobs N] [cargo-mutants options...]
#
#   parsing  the lexer, statements, expressions and the AST, the preprocessor,
#            the parser, materializing and validation, the streaming table and
#            parser, and results (testing ticket 04)
#   rest     merging, converting back to source, prompts, error hints, JSON,
#            the library, builtins, the crate's front page, the CLI, and the
#            binding crate (testing ticket 05)
#
# Results go to target/mutants/<group>/mutants.out/ (missed.txt, caught.txt,
# timeout.txt, outcomes.json). Mutants that change nothing are listed, with
# reasons, in .cargo/mutants.toml.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
group="${1:-}"
shift || true
case "$group" in
  parsing)
    files=(lexer statements expr ast preprocess parser materialize validation table stream result)
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
OPENUI_NO_LIVE_DIFFS=1 cargo mutants "${packages[@]}" "${args[@]}" --output "target/mutants/$group" "$@"
