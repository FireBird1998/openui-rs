#!/usr/bin/env python3
"""Measurement only (branches measure-parallel*, never merged).

Measures ways of running the OpenUIIntelligentUITests target on GitHub's
macOS runner. Three subcommands:

  classify LIST_FILE TEST_DIR OUT_FILE
      Read `swift test list` output, keep the OpenUIIntelligentUITests tests,
      classify each test's innermost suite as @MainActor or not by grepping
      the attributes on its declaration in TEST_DIR, and write one regular
      expression matching every @MainActor suite's tests to OUT_FILE.

  run LABEL LOG DEADLINE_SECONDS -- COMMAND...
      Print `=== MEASURE LABEL ===`, run COMMAND in its own process group,
      tee its output to stdout and LOG, stop it after DEADLINE_SECONDS, and
      read the wall time and the peak RSS of it and its waited descendants
      (ru_maxrss from wait4, bytes on macOS). Ends with one line
      `measure-result label=... wall=... maxrss=... exit=... deadline=...`.

  summarise LOG_DIR
      Parse every LOG_DIR/*.log and print the summary. A way's parts (way3a,
      way3b) are added together under their way (way3).
"""
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import sys
import time

TARGET = "OpenUIIntelligentUITests"


def classify(list_file, test_dir, out_file):
    suites = {}
    for line in Path(list_file).read_text(errors="replace").splitlines():
        line = line.strip()
        if not line.startswith(TARGET + "."):
            continue
        components = line[len(TARGET) + 1:].split("/")
        if len(components) < 2:
            continue
        # The last component is the function; the one before it is the
        # innermost suite (nested suites keep their own isolation in Swift).
        suites.setdefault(components[-2], []).append(line)

    sources = {path: path.read_text(errors="replace").splitlines()
               for path in sorted(Path(test_dir).rglob("*.swift"))}
    main_actor, nonisolated, unknown = [], [], []
    for suite in sorted(suites):
        declaration = re.compile(r"\b(?:struct|class|enum|actor)\s+" + re.escape(suite) + r"\b")
        attributes = None
        for path, lines in sources.items():
            for index, text in enumerate(lines):
                match = declaration.search(text)
                if not match or text.lstrip().startswith("//"):
                    continue
                # Attributes on the declaration line, plus whole attribute
                # lines directly above it (`@MainActor` then `@Suite(...)`).
                collected = [text[:match.start()]]
                above = index - 1
                while above >= 0 and lines[above].strip().startswith("@"):
                    collected.append(lines[above])
                    above -= 1
                attributes = " ".join(collected)
                break
            if attributes is not None:
                break
        if attributes is None:
            unknown.append(suite)
        elif re.search(r"@MainActor\b", attributes):
            main_actor.append(suite)
        else:
            nonisolated.append(suite)

    count = lambda names: sum(len(suites[name]) for name in names)
    print(f"measure-classify: {len(suites)} suites, {count(suites)} tests in {TARGET}")
    print(f"measure-classify: @MainActor {len(main_actor)} suites, {count(main_actor)} tests: {' '.join(main_actor)}")
    print(f"measure-classify: nonisolated {len(nonisolated)} suites, {count(nonisolated)} tests: {' '.join(nonisolated)}")
    if unknown:
        print(f"measure-classify: declaration not found (run with the nonisolated pass) "
              f"{len(unknown)} suites, {count(unknown)} tests: {' '.join(unknown)}")
    if not main_actor:
        print("measure-classify: no @MainActor suites found", file=sys.stderr)
        return 1
    # Type-name components only before the suite, so a function name, a
    # source location or a longer suite name (PolarChartScaleTests for
    # ChartScaleTests) never matches.
    regex = TARGET + r"\.([A-Za-z0-9_]+/)*(" + "|".join(map(re.escape, main_actor)) + ")/"
    Path(out_file).write_text(regex + "\n")
    print(f"measure-classify: @MainActor filter {regex}")
    return 0


def run(label, log, deadline_seconds, command):
    header = f"=== MEASURE {label} ===\n$ {' '.join(command)}\n"
    sys.stdout.write(header)
    sys.stdout.flush()
    Path(log).parent.mkdir(parents=True, exist_ok=True)
    with open(log, "wb") as out:
        out.write(header.encode())
        start = time.monotonic()
        deadline = start + deadline_seconds
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   stdin=subprocess.DEVNULL, start_new_session=True)
        fd = process.stdout.fileno()
        hit_deadline = False
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                hit_deadline = True
                break
            ready, _, _ = select.select([fd], [], [], min(remaining, 5))
            if not ready:
                continue
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            sys.stdout.buffer.write(chunk)
            sys.stdout.buffer.flush()
            out.write(chunk)
        waited = None

        def reap(until):
            # wait4, not Popen.poll, so the child's rusage is kept.
            nonlocal waited
            while waited is None:
                pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    waited = (status, usage)
                elif time.monotonic() >= until:
                    return
                else:
                    time.sleep(0.2)

        reap(deadline)
        if waited is None:
            hit_deadline = True
            for sig, grace in ((signal.SIGTERM, 10), (signal.SIGKILL, 30)):
                try:
                    os.killpg(process.pid, sig)
                except ProcessLookupError:
                    pass
                reap(time.monotonic() + grace)
                if waited is not None:
                    break
        else:
            # Kill anything the run left behind in its group.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
        wall = time.monotonic() - start
        if waited is not None:
            process.returncode = os.waitstatus_to_exitcode(waited[0])
            maxrss = str(waited[1].ru_maxrss)
        else:
            process.returncode = -9
            maxrss = "unknown"
        try:
            os.set_blocking(fd, False)
            rest = os.read(fd, 1 << 20)
        except OSError:
            rest = b""
        if rest:
            sys.stdout.buffer.write(rest)
            out.write(rest)
        result = (f"measure-result label={label} wall={wall:.1f} maxrss={maxrss} "
                  f"exit={process.returncode} deadline={int(hit_deadline)}\n")
        sys.stdout.write(result)
        sys.stdout.flush()
        out.write(result.encode())
    return 0 if process.returncode == 0 and not hit_deadline else 1


BODY = re.compile(r"watchdog-body (\d+(?:\.\d+)?) (.+)$")
UNARMED = re.compile(r"watchdog-body-unarmed (\d+(?:\.\d+)?) (.+)$")
RESULT = re.compile(r"measure-result label=(\S+) wall=([\d.]+) maxrss=(\S+) exit=(-?\d+) deadline=(\d)")
RUN_LINE = re.compile(r"Test run with (\d+) tests?(?: in (\d+) suites?)? (passed|failed)")


def summarise(log_dir):
    ways = {}
    for log in sorted(Path(log_dir).glob("*.log")):
        part = log.stem
        way = re.sub(r"[a-z]$", "", part) if re.fullmatch(r"way\d+[a-z]", part) else part
        entry = ways.setdefault(way, {"parts": [], "bodies": [], "unarmed": 0, "wall": 0.0,
                                      "rss": [], "watchdog": 0, "outer": 0, "time_limit": 0,
                                      "deadline": 0, "exits": [], "runs": []})
        entry["parts"].append(part)
        for line in log.read_text(errors="replace").splitlines():
            if match := BODY.search(line):
                entry["bodies"].append((float(match[1]), match[2].strip()))
            elif UNARMED.search(line):
                entry["unarmed"] += 1
            elif match := RESULT.search(line):
                entry["wall"] += float(match[2])
                if match[3] != "unknown":
                    entry["rss"].append(int(match[3]))
                entry["exits"].append(f"{match[1]}:{match[4]}")
                entry["deadline"] += int(match[5])
            elif "Swift test watchdog:" in line and "stack in" not in line and "did not produce" not in line \
                    and "could not write" not in line:
                if "outer limit:" in line:
                    entry["outer"] += 1
                else:
                    entry["watchdog"] += 1
            elif "Time limit was exceeded" in line:
                entry["time_limit"] += 1
            if match := RUN_LINE.search(line):
                entry["runs"].append(f"{part}:{match[1]} tests {match[3]}")

    lines = ["=== MEASURE SUMMARY ===",
             "way    parts        wall_s  peak_rss_mb  bodies  unarmed  max_body_s  over_30s  "
             "watchdog  outer  time_limit  deadline  exits"]
    for way, e in ways.items():
        bodies = sorted(e["bodies"], reverse=True)
        peak = f"{max(e['rss']) / 1048576:.0f}" if e["rss"] else "unknown"
        top = f"{bodies[0][0]:.1f}" if bodies else "-"
        over = sum(1 for seconds, _ in bodies if seconds > 30)
        lines.append(f"{way:<6} {'+'.join(e['parts']):<12} {e['wall']:>6.1f}  {peak:>11}  {len(bodies):>6}  "
                     f"{e['unarmed']:>7}  {top:>10}  {over:>8}  {e['watchdog']:>8}  {e['outer']:>5}  "
                     f"{e['time_limit']:>10}  {e['deadline']:>8}  {' '.join(e['exits'])}")
    for way, e in ways.items():
        lines.append(f"--- {way} test runs: {'; '.join(e['runs']) or 'no Swift Testing summary line'}")
        lines.append(f"--- {way} top 10 body times (s)")
        for seconds, name in sorted(e["bodies"], reverse=True)[:10]:
            lines.append(f"  {seconds:8.3f}  {name}")
    text = "\n".join(lines) + "\n"
    sys.stdout.write(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as out:
            out.write("```\n" + text + "```\n")
    return 0


def main(argv):
    if len(argv) >= 4 and argv[0] == "classify":
        return classify(*argv[1:4])
    if len(argv) >= 6 and argv[0] == "run" and argv[4] == "--":
        return run(argv[1], argv[2], float(argv[3]), argv[5:])
    if len(argv) == 2 and argv[0] == "summarise":
        return summarise(argv[1])
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
