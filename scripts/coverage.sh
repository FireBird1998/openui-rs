#!/usr/bin/env bash
# Line coverage of the core and the binding over the offline suite, the tests
# scripts/check.sh runs. Prints a summary per file, writes an HTML report to
# target/llvm-cov/html, and fails if a line no test runs isn't explained in
# crates/openui-lang/UNREACHED.md. Needs cargo-llvm-cov and the
# llvm-tools-preview component.
#
#   scripts/coverage.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export OPENUI_NO_LIVE_DIFFS=1
cargo llvm-cov --workspace --no-report
cargo llvm-cov report --summary-only
cargo llvm-cov report --html >/dev/null
echo "HTML report: target/llvm-cov/html/index.html"

# Every uncovered line must be listed, by its text, under its file.
cargo llvm-cov report --show-missing-lines 2>/dev/null \
  | sed -n '/^Uncovered Lines:/,$p' | tail -n +2 \
  | python3 -I -c '
import re, sys
root, explained_path = sys.argv[1], sys.argv[2]
explained, section = {}, None
for line in open(explained_path, encoding="utf-8"):
    if line.startswith("## "):
        section = line[3:].strip()
        explained.setdefault(section, set())
    elif section:
        for span in re.findall(r"`([^`]+)`", line):
            explained[section].add(span.strip())
    # Spans that contain backticks (a Markdown fence) are matched loosely below.
unexplained = []
for entry in sys.stdin:
    path, _, spans = entry.strip().partition(": ")
    if not spans:
        continue
    rel = path.split(root + "/", 1)[-1]
    source = open(path, encoding="utf-8").read().split("\n")
    allowed = explained.get(rel, set())
    for span in spans.split(", "):
        first, _, last = span.partition("-")
        for n in range(int(first), int(last or first) + 1):
            text = source[n - 1].strip()
            if text in allowed or any(text.replace("`", "") == a.replace("`", "") for a in allowed):
                continue
            # The fenced fallback in preprocess.rs holds backticks of its own.
            if "```" in text and any(a.startswith("return match body.rfind(") for a in allowed):
                continue
            unexplained.append(f"{rel}:{n}: {text}")
if unexplained:
    print("Lines no test runs that crates/openui-lang/UNREACHED.md does not explain:")
    print("\n".join("  " + u for u in unexplained))
    sys.exit(1)
print("Every line no test runs is explained in crates/openui-lang/UNREACHED.md.")
' "$ROOT" crates/openui-lang/UNREACHED.md
