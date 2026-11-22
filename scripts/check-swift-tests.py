#!/usr/bin/env python3
"""Run a complete Swift Testing package; exit zero only with a complete passing run.

Usage: python3 scripts/check-swift-tests.py PACKAGE [SwiftPM build options...]
Expected tests are discovered with `swift test list` on every invocation.
"""
import collections
import re
import subprocess
import sys


class IncompleteRun(ValueError):
    pass


def declared_tests(output):
    tests = [line.strip() for line in output.splitlines() if line.strip()]
    if not tests or any(not re.fullmatch(r"\w+\.[^\s]+\([^\n]*\)", name) for name in tests):
        raise IncompleteRun("swift test list did not produce a nonempty test inventory")
    if len(set(tests)) != len(tests):
        raise IncompleteRun("swift test list returned duplicate test IDs")
    return tests


def audit(tests, output):
    """Check both started-test names and the runner's final summary, even on failure."""
    expected = collections.Counter(name.split(".", 1)[1].rsplit("/", 1)[-1] for name in tests)
    started = collections.Counter(re.findall(r"Test (\S+\([^\n]*?\)) started\.", output))
    summaries = re.findall(r"Test run with (\d+) tests?(?: in \d+ suites?)? (passed|failed)\b", output)
    problems = []
    if started != expected:
        missing = list((expected - started).elements())
        extra = list((started - expected).elements())
        problems.append(f"started {sum(started.values())}/{len(tests)} declared tests; missing={missing}, unexpected={extra}")
    # SwiftPM runs each test target on its own, each ending with its own summary.
    targets = collections.Counter(name.split(".", 1)[0] for name in tests)
    if len(targets) == 1 and len(summaries) != 1:
        problems.append(f"expected one final test summary, found {len(summaries)}")
    elif len(summaries) != len(targets):
        problems.append(f"expected {len(targets)} final test summaries (one per test target), found {len(summaries)}")
    elif len(targets) == 1 and int(summaries[0][0]) != len(tests):
        problems.append(f"summary reports {summaries[0][0]}/{len(tests)} declared tests")
    elif sorted(int(count) for count, _ in summaries) != sorted(targets.values()):
        problems.append(f"summaries report {sorted(int(count) for count, _ in summaries)} tests; "
                        f"the test targets declare {sorted(targets.values())}")
    if problems:
        raise IncompleteRun("; ".join(problems))
    return "passed" if all(result == "passed" for _, result in summaries) else "failed"


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    package, *options = argv
    # This is a whole-package gate. Filters/repetitions cannot weaken its inventory.
    forbidden = {"--filter", "--skip", "--specifier", "-s", "--list-tests", "-l",
                 "--maximum-repetitions", "--repeat-until", "--disable-swift-testing",
                 "--package-path", "--debugger", "list", "last"}
    if any(option.split("=", 1)[0] in forbidden for option in options):
        print("Swift test gate: filtering, repetition and alternate test commands are not allowed", file=sys.stderr)
        return 2
    command = ["swift", "test", "--package-path", package, *options]
    listed = subprocess.run([*command, "list"], text=True, stdout=subprocess.PIPE)
    if listed.returncode:
        return listed.returncode
    try:
        tests = declared_tests(listed.stdout)
    except IncompleteRun as error:
        print(f"Swift test gate: {error}", file=sys.stderr)
        return 1
    print(f"Swift test gate: {package} declares {len(tests)} tests", flush=True)
    process = subprocess.Popen(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    lines = []
    try:
        for line in process.stdout:
            print(line, end="", flush=True)
            lines.append(line)
        status = process.wait()
    except BaseException:
        process.terminate()
        process.wait()
        raise
    try:
        result = audit(tests, "".join(lines))
    except IncompleteRun as error:
        print(f"Swift test gate: INCOMPLETE: {error} (runner exit {status})", file=sys.stderr)
        return 1
    print(f"Swift test gate: complete {len(tests)}/{len(tests)}; summary {result}; runner exit {status}", flush=True)
    return 0 if status == 0 and result == "passed" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
