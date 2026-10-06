#!/usr/bin/env bash
# Streaming benchmarks: the core against the reference parser on the same
# workload (see `openui-lang bench` and reference/bench.ts). Needs Bun.
#
#   scripts/bench.sh            # the 7 benchmark samples and synthetic programs up to 32 copies
#   BENCH_RUNS=3 BENCH_COPIES="2 4" scripts/bench.sh   # fewer runs, smaller synthetic programs
#
# Prints a markdown table; BENCHMARKS.md records the last run.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
(cd reference && bun install --silent)
cargo build -q --release -p openui-lang

schema=conformance/imported/schemas/bench.json
samples=(simple-table chart-with-data contact-form settings-panel dashboard e-commerce-product pricing-page)
files=()
for s in "${samples[@]}"; do files+=("conformance/imported/cases/bench/$s.oui"); done
synth=target/bench
mkdir -p "$synth"
for k in ${BENCH_COPIES:-2 4 8 16 32}; do
  bun reference/synth.ts conformance/imported/cases/bench/dashboard.oui "$k" "$synth/dashboard-x$k.oui"
  files+=("$synth/dashboard-x$k.oui")
done

rust="$(target/release/openui-lang bench "$schema" "${files[@]}")"
ref="$(cd reference && bun bench.ts "../$schema" "${files[@]/#/../}")"

echo "| Program | Code points | Pushes | Reference (ms) | Core (ms) | Speedup |"
echo "|---|---:|---:|---:|---:|---:|"
paste <(echo "$ref") <(echo "$rust") | while IFS=$'\t' read -r name cps pushes ref_ms _ _ _ rust_ms; do
  speedup="$(awk -v a="$ref_ms" -v b="$rust_ms" 'BEGIN { if (b > 0) printf "%.1f×", a / b; else print "–" }')"
  echo "| ${name%.oui} | $cps | $pushes | $ref_ms | $rust_ms | $speedup |"
done
