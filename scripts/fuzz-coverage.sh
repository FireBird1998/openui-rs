#!/usr/bin/env bash
# Coverage-guided fuzzing (cargo-fuzz, on the nightly toolchain): runs the
# targets in fuzz/fuzz_targets/ from seeds taken out of the conformance corpus,
# with the dictionary in fuzz/openui.dict. A crash, a panic or an input over the
# time limit stops a target and leaves the input in its artifacts folder.
#
#   scripts/fuzz-coverage.sh [--seconds N] [--findings DIR] [target...]
#
# With no targets it runs them all, in parallel. --seconds bounds each target
# (default 60); a full night is about 28800. --findings puts the artifacts and
# logs in DIR, as hunts do, instead of fuzz/artifacts/<target>/.
#
# Inputs stay under 8 KB, the size of a long model response: smaller inputs
# run faster, so the fuzzer explores more. The time limit is 10 s an input: the core answers any input in well under
# a second in a release build, and a fuzz build adds AddressSanitizer and debug
# assertions, which make it several times slower. Ten seconds still catches
# what grows without bound, which is what the limit is for.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

seconds=60
findings=""
targets=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --seconds) seconds="$2"; shift 2 ;;
    --findings) findings="$2"; shift 2 ;;
    -*) echo "usage: scripts/fuzz-coverage.sh [--seconds N] [--findings DIR] [target...]" >&2; exit 2 ;;
    *) targets+=("$1"); shift ;;
  esac
done
[[ ${#targets[@]} -gt 0 ]] || targets=(parse stream merge serialize json)

cargo +nightly fuzz build > /dev/null 2>&1 || { cargo +nightly fuzz build; exit 1; }

# Seeds: corpus programs for parsing and streaming (a stream's first byte is
# its chunk plan), program-and-patch pairs for merging, and parse results for
# converting back to source and for JSON.
seed() {
  local target="$1" dir="fuzz/corpus/$1"
  mkdir -p "$dir"
  [[ -z "$(ls -A "$dir")" ]] || return 0
  case "$target" in
    parse) find conformance/cases conformance/imported/cases -name '*.oui' -size -8k -exec cp {} "$dir/" \; ;;
    stream)
      find conformance/cases conformance/imported/cases -name '*.oui' -size -8k | while read -r f; do
        { printf '\007'; cat "$f"; } > "$dir/$(basename "$f")"
      done ;;
    merge)
      python3 -I - "$dir" conformance/merge.json conformance/imported/merge.json <<'PY'
import hashlib, json, sys
out = sys.argv[1]
for path in sys.argv[2:]:
    for case in json.load(open(path))["cases"]:
        if "existing" in case and "patch" in case:
            data = (case["existing"] + "\0" + case["patch"]).encode()
            open(f"{out}/{hashlib.sha1(data).hexdigest()[:12]}", "wb").write(data)
PY
      ;;
    serialize | json) find conformance/golden -name '*.oneshot.json' -size -64k -exec cp {} "$dir/" \; ;;
  esac
}

pids=()
for target in "${targets[@]}"; do
  seed "$target"
  out="${findings:-fuzz/artifacts/$target}"
  mkdir -p "$out"
  prefix="$out/"
  [[ -z "$findings" ]] || prefix="$out/coverage-$target-"
  echo "Fuzzing $target for $seconds s"
  cargo +nightly fuzz run "$target" "fuzz/corpus/$target" -- \
    -dict=fuzz/openui.dict -timeout=10 -rss_limit_mb=4096 -max_len=8192 -max_total_time="$seconds" \
    -print_final_stats=1 -artifact_prefix="$prefix" > "$out/coverage-$target.log" 2>&1 &
  pids+=($!)
done
status=0
for i in "${!pids[@]}"; do
  target="${targets[$i]}"
  log="${findings:-fuzz/artifacts/$target}/coverage-$target.log"
  if wait "${pids[$i]}"; then
    result="clean"
  else
    result="FOUND SOMETHING (see $log)"
    status=1
  fi
  runs="$(grep -o 'stat::number_of_executed_units: *[0-9]*' "$log" | grep -o '[0-9]*$' || echo '?')"
  cov="$(grep -o 'cov: [0-9]*' "$log" | tail -1 | grep -o '[0-9]*' || echo '?')"
  echo "$target: $runs inputs, coverage $cov, $result"
done
exit $status
