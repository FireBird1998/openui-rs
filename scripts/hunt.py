"""Helpers for scripts/hunt.sh and scripts/upstream-check.sh. Standard library only.

  python3 -I scripts/hunt.py compare-goldens <old> <new> <out.json>
      Which cases' outputs differ between two golden folders. Deletes the
      files under <new> that match <old>, so <new> keeps only the changes.
  python3 -I scripts/hunt.py packages <lang-core> <pnpm-workspace.yaml> <our package.json>
      Prints a package.json for running that lang-core: the packages its
      source imports at run time, at the versions its package.json asks for
      (resolving pnpm's "catalog:"), plus our reference's own.
  python3 -I scripts/hunt.py report <hunt-dir> <seconds-it-ran>
      Writes <hunt-dir>/report.md from what the hunt's checks left there.
"""

import json
import re
import sys
from pathlib import Path

PARTS = ["oneshot", "steps", "tokens", "statements", "preprocess"]
EDIT_KINDS = {"merge": "merges", "serialize": "conversions back to source", "enrich": "error hints"}


def files(root):
    return {p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()}


def compare_goldens(old, new, out):
    old, new = Path(old), Path(new)
    old_files, new_files = files(old), files(new)
    read = lambda root, rel: (root / rel).read_bytes() if (root / rel).is_file() else None
    parse, edits, prompts = {}, {}, []
    totals = {
        "parse": sum(1 for f in old_files if f.endswith(".oneshot.json")),
        "prompts": sum(1 for f in old_files if f.startswith("prompts/")),
    }
    for kind in EDIT_KINDS:
        golden = read(old, f"edits/{kind}.json")
        totals[kind] = len(json.loads(golden)) if golden else 0
    for rel in sorted(old_files | new_files):
        before, after = read(old, rel), read(new, rel)
        if rel.startswith("edits/"):
            a = json.loads(before) if before else {}
            b = json.loads(after) if after else {}
            names = sorted(k for k in a.keys() | b.keys() if a.get(k) != b.get(k))
            if names:
                edits[Path(rel).stem] = names
            changed = bool(names)
        else:
            changed = before != after
            if changed and rel.startswith("prompts/"):
                prompts.append(rel[len("prompts/") : -len(".json")])
            elif changed:
                case, part = rel[: -len(".json")].rsplit(".", 1)
                parse.setdefault(case, []).append(part)
        if not changed and after is not None:
            (new / rel).unlink()
    for folder in sorted((p for p in new.rglob("*") if p.is_dir()), key=lambda p: -len(p.parts)):
        if not any(folder.iterdir()):
            folder.rmdir()
    summary = {"totals": totals, "parse": parse, "edits": edits, "prompts": prompts}
    Path(out).write_text(json.dumps(summary, indent=1) + "\n")


IMPORT = re.compile(
    r"""^\s*(?:import|export)\s+(?!type\b)[^;]*?\bfrom\s+["']([^"'.][^"']*)["']"""
    r"""|^\s*import\s+["']([^"'.][^"']*)["']"""
    r"""|\bimport\(\s*["']([^"'.][^"']*)["']\s*\)""",
    re.M,
)


def catalog(workspace):
    """pnpm's default catalog: package name to version range."""
    versions, inside = {}, False
    for line in Path(workspace).read_text().splitlines() if Path(workspace).is_file() else []:
        if not line.startswith(" "):
            inside = line.strip() == "catalog:"
        elif inside and ":" in line:
            name, _, version = line.strip().rpartition(": ")
            versions[name.strip("\"'")] = version.strip("\"'")
    return versions


def packages(lang_core, workspace, ours):
    names = set()
    for source in (Path(lang_core) / "src").rglob("*.ts"):
        if "__tests__" in source.parts:
            continue
        for match in IMPORT.finditer(source.read_text()):
            spec = next(group for group in match.groups() if group)
            if spec.startswith(("node:", "bun:")):
                continue
            parts = spec.split("/")
            names.add("/".join(parts[:2]) if spec.startswith("@") else parts[0])
    # A vendored copy keeps upstream's package.json as package.json.upstream.
    manifest_path = Path(lang_core) / "package.json"
    if not manifest_path.is_file():
        manifest_path = Path(lang_core) / "package.json.upstream"
    manifest = json.loads(manifest_path.read_text())
    declared = {**manifest.get("peerDependencies", {}), **manifest.get("dependencies", {})}
    versions = catalog(workspace)
    dependencies = dict(json.loads(Path(ours).read_text()).get("dependencies", {}))
    for name in sorted(names):
        version = declared.get(name, "latest")
        if version.startswith("catalog:"):
            version = versions.get(name, "latest")
        dependencies[name] = version
    print(json.dumps({"name": "upstream-check", "private": True, "type": "module", "dependencies": dependencies}, indent=1))


# ── report ──────────────────────────────────────────────────────────────────


def text(path, default=""):
    return path.read_text().strip() if path.is_file() else default


def number(n):
    return f"{n:,}"


def duration(seconds):
    minutes, seconds = divmod(int(seconds), 60)
    return f"{minutes} min {seconds} s" if minutes else f"{seconds} s"


def fenced(content):
    ticks = max([3] + [len(run) + 1 for run in re.findall(r"`+", content)])
    return f"{'`' * ticks}\n{content}\n{'`' * ticks}"


def shown(content):
    """A fenced block of the text, with its exact form as a JSON string when it
    holds characters a block doesn't show."""
    block = fenced(content)
    if re.search(r"[^\S\n ]|[\x00-\x1f\x7f  ]", content.replace("\n", "")):
        block += f"\n\nExactly, as a JSON string: `{json.dumps(content)}`"
    return block


PLURALS = {"mismatch": "mismatches", "crash": "crashes", "blow-up": "blow-ups", "quirk?": "quirks?"}


def tally(findings):
    counts = {}
    for f in findings:
        counts[f["kind"]] = counts.get(f["kind"], 0) + 1
    return counts


def kinds_line(counts):
    return ", ".join(f"{counts[k]} {k if counts[k] == 1 else PLURALS[k]}" for k in PLURALS if counts.get(k))


def fuzz_row(name, unit, summary, findings):
    if summary is None:
        return None
    counts = tally(findings)
    found = kinds_line(counts) or "none"
    explained = summary.get("explained", 0)
    if explained:
        found += f" ({explained} known divergence{'s' if explained != 1 else ''})"
    unrecorded = sum(summary.get(k, 0) for k in ["mismatch", "crash", "blow-up", "quirk?"]) - explained - sum(counts.values())
    if unrecorded > 0:
        found += f"; {unrecorded} more not shrunk or repeats"
    return f"| {name} | {number(summary['inputs'])} {unit} | {found} |"


def finding_section(n, f):
    check = {"parse": "parsing", "edits": "edits", "merge": "merging", "serialize": "converting back to source",
             "invariant": f"the {f.get('rule', '')} invariant", "invariants": "invariants",
             "coverage": f"coverage-guided fuzzing, {f.get('target', '')}"}[f["check"]]
    kind = {"mismatch": "Mismatch", "crash": "Crash", "blow-up": "Blow-up", "quirk?": "Quirk?"}[f["kind"]]
    lines = [f"### {n}. {kind}: {check}" + (", streaming" if f.get("streaming") else "")]
    if "seed" in f:
        where = f"seed {f['seed']}, {'program' if f['check'] == 'parse' else 'input'} {f['index']}"
        if f.get("library"):
            where += f"; library `{f['library']}`"
        if f.get("root"):
            where += f", root `{f['root']}`"
        lines.append(f"- **Where:** {where}")
    if f["kind"] == "blow-up" and "core_ms" in f:
        ref = f" (the reference took {f['reference_ms']:.0f} ms)" if "reference_ms" in f else ""
        lines.append(f"- **Time:** the core took {f['core_ms']:.0f} ms{ref}")
    if f["check"] in ("parse", "invariant") and "chars" in f:
        before, after = f["chars"]
        if f["kind"] == "blow-up":
            lines.append(f"- **Not shrunk:** blow-ups aren't. {number(before)} characters.")
        elif f.get("shrunk"):
            lines.append(f"- **Shrunk:** from {number(before)} to {number(after)} characters")
        else:
            lines.append(f"- **Partly shrunk:** time ran out, at {number(after)} of {number(before)} characters")
    elif f["check"] == "merge" and not f.get("shrunk"):
        lines.append("- **Partly shrunk:** time ran out" if f["kind"] != "blow-up" else "- **Not shrunk:** blow-ups aren't")
    if f.get("replay"):
        lines.append(f"- **Replay:** `{f['replay']}`")
    if f.get("case"):
        where = f"`cases/{f['case']}.oui`, listed in `fuzz.json`" if f["check"] == "parse" else f"`{f['file']}`"
        lines.append(f"- **Saved as:** `{f['case']}` in this folder: {where}")
    elif f["kind"] == "quirk?" and f["check"] == "parse":
        lines.append("- **Not saved as a case:** the reference throws, so it has no golden")
    if isinstance(f.get("reference"), str):
        lines.append(f"- **The reference threw:** `{f['reference']}`")
    if f.get("note"):
        lines.append(f"- {f['note']}")
    lines.append("")
    if f["check"] in ("parse", "invariant", "coverage") and "input" in f:
        lines.append(shown(f["input"]))
    if f["check"] == "invariant":
        lines += ["", "The core:", "", fenced(json.dumps(f.get("core"), ensure_ascii=False)),
                  "", "The reference:", "", fenced(json.dumps(f.get("reference"), ensure_ascii=False))]
    elif f["check"] == "merge" and "input" in f:
        lines += ["Existing program:", "", shown(f["input"]["existing"]), "", "Patch:", "", shown(f["input"]["patch"])]
        if "rootId" in f["input"]:
            lines += ["", f"Root id: `{json.dumps(f['input']['rootId'])}`"]
    elif f["check"] == "serialize" and "input" in f:
        lines += ["Element and state:", "", fenced(json.dumps(f["input"], indent=1, ensure_ascii=False))]
    if f["check"] in ("merge", "serialize") and "input" in f:
        lines += ["", "The reference:", "", fenced(json.dumps(f.get("reference"), ensure_ascii=False))]
        lines += ["", "The core:", "", fenced(json.dumps(f.get("core"), ensure_ascii=False))]
    if f.get("log"):
        lines += ["The end of its log:", "", fenced(f["log"])]
    return "\n".join(lines)


COVERAGE_TARGETS = ["parse", "stream", "merge", "serialize", "json"]


def coverage_findings(hunt):
    """What coverage-guided fuzzing left: an input per crash, timeout or memory blow-up."""
    out = []
    for target in COVERAGE_TARGETS:
        for kind, label in [("crash", "crash"), ("timeout", "blow-up"), ("oom", "blow-up"), ("leak", "crash")]:
            for path in sorted(hunt.glob(f"coverage-{target}-{kind}-*")):
                data = path.read_bytes()
                out.append({"kind": label, "check": "coverage", "target": target, "file": path.name,
                            "input": data.decode("utf-8", "replace"),
                            "replay": f"cargo +nightly fuzz run {target} {path}"})
    return out


def coverage_row(hunt):
    """The coverage-guided targets' row: inputs run, and the coverage reached."""
    runs, parts = 0, []
    for target in COVERAGE_TARGETS:
        log = text(hunt / f"coverage-{target}.log")
        executed = re.findall(r"stat::number_of_executed_units: *(\d+)", log)
        cov = re.findall(r"cov: (\d+)", log)
        if executed:
            runs += int(executed[-1])
            parts.append(f"{target} {cov[-1] if cov else '?'}")
    if not parts:
        return None
    return f"{number(runs)} inputs (coverage: {', '.join(parts)})"


def process_findings(hunt, check, label):
    """Findings for a fuzzer that died or had to be stopped, from its status."""
    status = text(hunt / f"status-{check}")
    replay = text(hunt / f"progress-{check}", "")
    log_lines = text(hunt / f"{check}.log").splitlines()[-15:]
    base = {"check": check, "replay": replay, "log": "\n".join(log_lines)}
    if status == "killed":
        return [{**base, "kind": "blow-up", "note": f"The {label} fuzzer was still running when its time was up, so the hunt stopped it. An input in its last batch probably hangs the core or the reference; the replay runs that batch."}]
    if status.startswith("exit"):
        code = int(status.split()[1])
        how = f"signal {code - 128}" if code > 128 else f"status {code}"
        return [{**base, "kind": "crash", "note": f"The {label} fuzzer stopped with {how}: a crash the core couldn't catch, or the reference process died. The replay runs its last batch."}]
    return []


def upstream_section(hunt, meta):
    up = hunt / "upstream"
    status = text(hunt / "status-upstream")
    pin = f"`{meta['pin'][:7]}` (@openuidev/lang-core {meta['version']})"
    lines = ["## Upstream", ""]
    if status.startswith("skipped"):
        return lines + [f"Not checked: {status[len('skipped: '):]}."], "not checked"
    state = text(up / "status")
    source = text(up / "source")
    if status == "killed":
        lines.append("The check didn't finish within the hunt's time. Run `scripts/upstream-check.sh <folder>` alone to finish it.\n")
    if state == "unreachable" or not state:
        return lines + [f"Not checked: upstream's history came from {source or 'nowhere'}."], "not checked"
    main = text(up / "main")[:7]
    lines.append(f"Pinned at {pin}. Upstream's main is at `{main}`, from {source}.")
    lines.append("")
    if state == "not-on-main":
        lines.append("**The pinned commit isn't on upstream's main any more** (a rewritten history?). Moving the pin needs a look by hand.")
        return lines, "the pin isn't on main"
    if state == "current":
        lines.append("No lang-core commits since the pin.")
        return lines, "no new lang-core commits"
    commits = [line.split("\t", 2) for line in text(up / "commits.tsv").splitlines()]
    lines.append(f"**{len(commits)} lang-core commit{'s' if len(commits) != 1 else ''} since the pin**, newest first:")
    lines.append("")
    lines += [f"- {date} `{sha[:7]}` {subject}" for sha, date, subject in commits[:20]]
    if len(commits) > 20:
        lines.append(f"- … and {len(commits) - 20} more, in `upstream/commits.tsv`")
    lines.append("")
    goldens = text(up / "goldens")
    newest = text(up / "newest").split()
    newest_name = f"`{newest[0][:7]}` (lang-core {newest[1] if len(newest) > 1 else '?'})" if newest else f"`{commits[0][0][:7]}`"
    row = f"{len(commits)} new lang-core commit{'s' if len(commits) != 1 else ''}"
    if goldens != "computed":
        reason = goldens[len("not computed: "):] if goldens.startswith("not computed: ") else "the check didn't get that far"
        lines.append(f"Our corpus's outputs with {newest_name} weren't computed: {reason}.")
        return lines, row + "; outputs not computed"
    changed = json.loads((up / "changed.json").read_text())
    totals = changed["totals"]
    parts = {}
    for case_parts in changed["parse"].values():
        for part in case_parts:
            parts[part] = parts.get(part, 0) + 1
    counts = [
        ("parse cases", len(changed["parse"]), totals["parse"]),
        *[(label, len(changed["edits"].get(kind, [])), totals[kind]) for kind, label in EDIT_KINDS.items()],
        ("system prompts", len(changed["prompts"]), totals["prompts"]),
    ]
    if not any(n for _, n, _ in counts):
        lines.append(f"With {newest_name}, every output across our corpus stays the same.")
        return lines, row + "; no output changes"
    lines.append(f"With {newest_name}, outputs change for:")
    lines.append("")
    for label, n, total in counts:
        if not n:
            continue
        detail = ""
        if label == "parse cases":
            labels = {"oneshot": "one-shot", "steps": "streaming", "tokens": "tokens", "statements": "statements", "preprocess": "preprocessing"}
            detail = " (" + ", ".join(f"{labels[p]} {parts[p]}" for p in PARTS if p in parts) + ")"
        lines.append(f"- {n} of {number(total)} {label}{detail}")
    lines.append("")
    named = [f"`{c}`" for c in sorted(changed["parse"])]
    named += [f"`{kind}/{name}`" for kind, names in changed["edits"].items() for name in names]
    named += [f"`prompts/{p}`" for p in changed["prompts"]]
    lines.append("Changed: " + ", ".join(named[:40]) + (f", and {len(named) - 40} more" if len(named) > 40 else "") + ".")
    lines.append("")
    lines.append("`upstream/changed.json` lists them all, `upstream/golden/` holds the new versions of the changed goldens, and `upstream/lang-core/` the new reference. Moving the pin is its own ticket.")
    summary = ", ".join(f"{n} {label}" for label, n, _ in counts if n)
    return lines, row + f"; outputs change for {summary}"


def report(hunt, ran):
    hunt = Path(hunt)
    meta = json.loads((hunt / "meta.json").read_text())
    findings = []
    if (hunt / "findings.jsonl").is_file():
        findings = [json.loads(line) for line in (hunt / "findings.jsonl").read_text().splitlines() if line.strip()]
    findings += process_findings(hunt, "parse", "parsing") + process_findings(hunt, "edits", "edits") + process_findings(hunt, "invariants", "invariants")
    findings += coverage_findings(hunt)
    summaries = {c: json.loads(text(hunt / f"summary-{c}.json")) if (hunt / f"summary-{c}.json").is_file() else None for c in ["parse", "merge", "serialize", "invariants"]}

    code = f"`{meta['commit'][:7]}`" + (" with uncommitted changes" if meta["dirty"] else "")
    out = [f"# Hunt, {meta['started']}", ""]
    out.append(f"Seed {meta['seed']}, {meta['minutes']} minutes (ran {duration(ran)}), on {code}, against @openuidev/lang-core {meta['version']} at `{meta['pin'][:7]}`.")
    out.append("")
    out += ["## Summary", "", "| Check | Ran | Findings |", "|---|---|---|"]
    by_check = lambda c: [f for f in findings if f["check"] == c]
    for name, check, unit in [("Differential fuzzing, parsing", "parse", "programs"), ("Differential fuzzing, merging", "merge", "merges"), ("Differential fuzzing, converting back to source", "serialize", "trees")]:
        row = fuzz_row(name, unit, summaries[check], by_check(check))
        if row:
            out.append(row)
        elif by_check(check):
            out.append(f"| {name} | stopped early | {kinds_line(tally(by_check(check)))} |")
    for rule, tallies in (summaries["invariants"] or {}).items():
        found = [f for f in by_check("invariant") if f.get("rule") == rule]
        row = fuzz_row(f"Invariant: {rule.replace('-', ' ')}", "programs", tallies, found)
        out.append(row.replace(" known divergence", " known exception"))
    for check, label in [("parse", "parsing"), ("edits", "edits"), ("invariants", "invariants")]:
        status = text(hunt / f"status-{check}")
        if status.startswith("skipped"):
            out.append(f"| Differential fuzzing, {label} | not run: {status[len('skipped: '):]} | |")
        elif status != "ok" and check == "edits" and by_check("edits"):
            out.append(f"| Differential fuzzing, edits | stopped early | {kinds_line(tally(by_check('edits')))} |")
    coverage = coverage_row(hunt)
    status = text(hunt / "status-coverage")
    if coverage:
        found = kinds_line(tally(by_check("coverage"))) or "none"
        out.append(f"| Coverage-guided fuzzing | {coverage} | {found} |")
    elif status.startswith("skipped"):
        out.append(f"| Coverage-guided fuzzing | not run: {status[len('skipped: '):]} | |")
    upstream_lines, upstream_row = upstream_section(hunt, meta)
    out.append(f"| Upstream | {upstream_row} | |")
    out.append("")
    counts = tally(findings)
    if findings:
        out.append(f"**{len(findings)} finding{'s' if len(findings) != 1 else ''}:** {kinds_line(counts)}.")
    else:
        out.append("**Nothing found.**")
    slowest = summaries["parse"]
    if slowest and "slowest_ms" in slowest:
        out.append("")
        out.append(f"The slowest program took the core {slowest['slowest_ms']:.0f} ms (program {slowest['slowest_index']}); over {meta['slow_ms']} ms is a blow-up.")
    tracked = text(hunt / "tracked-changes.txt")
    if tracked:
        out += ["", "**Tracked files changed during the hunt.** A hunt never changes them, so something else did, or the hunt has a bug:", "", fenced(tracked)]
    if findings:
        out += ["", "## Findings", ""]
        for n, f in enumerate(findings, 1):
            out += [finding_section(n, f), ""]
        out.append("Replays generate from the corpus as it is now, so they reproduce only while the corpus is unchanged; the saved cases keep each finding regardless. TESTING.md says how to sort a finding and turn it into a case.")
    out += ["", *upstream_lines, ""]
    out += [
        "## In this folder",
        "",
        "- `report.md`: this report",
        "- `findings.jsonl`: each finding as a line of JSON",
        "- `fuzz.json` with `cases/fuzz/`, `merge-fuzz.json`, `serialize-fuzz.json`: the findings saved as corpus cases, in the corpus's formats",
        "- `parse.log`, `edits.log`, `upstream.log`: what each check printed",
        "- `upstream/`: the upstream check's working files",
        "",
    ]
    (hunt / "report.md").write_text("\n".join(out))


if __name__ == "__main__":
    if len(sys.argv) == 5 and sys.argv[1] == "compare-goldens":
        compare_goldens(*sys.argv[2:])
    elif len(sys.argv) == 5 and sys.argv[1] == "packages":
        packages(*sys.argv[2:])
    elif len(sys.argv) == 4 and sys.argv[1] == "report":
        report(sys.argv[2], float(sys.argv[3]))
    else:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
