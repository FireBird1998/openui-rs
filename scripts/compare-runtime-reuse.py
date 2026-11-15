#!/usr/bin/env python3
"""Assert reuse_measurements work/output equivalence; report peak allocations.

Build main with `git archive db80c86` in its own directory AND target directory,
copy the example and tests/support/reuse.rs there, then run on both revisions:
    cargo run --offline --release -p openui-runtime --example reuse_measurements -- costs 9216
Compare the two TSVs with this script. Peak allocation ratios are descriptive; the enforced comparisons are work
and serialized output. The six-column mixed_prop_measurements generator TSVs
are also accepted (they contain no peak column).
"""
import argparse
import json
from pathlib import Path


def read(path):
    return [line.split("\t") for line in Path(path).read_text().splitlines()]


def compare(before, after):
    assert len(before) == len(after) and before, "different/empty measurement sets"
    mixed = len(after[0]) == 6
    source_column = 5 if mixed else 7
    result = {
        "evaluations": len(after),
        "distinct_sources": len({row[source_column] for row in after}),
        "output_hash_mismatches": 0,
    }
    for old, new in zip(before, after):
        assert old[:2] == new[:2] and old[source_column] == new[source_column], "different programs"
        if not mixed:
            assert old[4] == new[4] == "false", "generator case was cut"
        result["output_hash_mismatches"] += (old[3:5] != new[3:5] if mixed else old[5:7] != new[5:7])
    for column, name in ([(2, "work")] if mixed else [(2, "work"), (3, "peak")]):
        ratios = [(int(new[column]) / int(old[column]), new, old)
                  for old, new in zip(before, after)]
        ratio, new, old = max(ratios, key=lambda item: item[0])
        result[name] = {
            "increases": sum(item[0] > 1 for item in ratios),
            "reduced": sum(item[0] < 1 for item in ratios),
            "max_ratio": ratio,
            "max_case": {"seed": int(new[0]), "data": new[1] == "true",
                         "after": int(new[column]), "main": int(old[column]),
                         "source": json.loads(new[source_column])},
        }
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before")
    parser.add_argument("after")
    args = parser.parse_args()
    summary = compare(read(args.before), read(args.after))
    print(json.dumps(summary, indent=2))
    assert summary["output_hash_mismatches"] == 0, "generated output changed"
    assert summary["work"]["increases"] == 0, "a generated program needs more work than main"
