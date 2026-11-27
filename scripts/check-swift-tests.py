#!/usr/bin/env python3
"""Run a complete Swift Testing package; exit zero only with a complete passing run.

Usage: python3 scripts/check-swift-tests.py PACKAGE [SwiftPM build options...]
Expected tests are discovered with `swift test list` on every invocation.
OpenUINative runs as one `swift test` per test target (see NATIVE_RUNS);
other packages run in one.
"""
import collections
from contextlib import nullcontext
import ctypes
import json
import os
from pathlib import Path
import re
import select
import selectors
import signal
import subprocess
import sys
import tempfile
import time


NATIVE_PACKAGE = Path(__file__).resolve().parents[1] / "swift/OpenUINative"
CHAT_PACKAGE = Path(__file__).resolve().parents[1] / "tools/chat"
# MCPFixture mints these synthetic access and refresh tokens. Check both
# child streams before relaying them, including output from successful tests.
MCP_TEST_TOKEN = re.compile(rb"synthetic-(?:access|refresh)-[0-9]+")
MCP_TEST_PREFIXES = (b"synthetic-access-", b"synthetic-refresh-")
MCP_TEST_PARTIALS = {prefix[:length] for prefix in MCP_TEST_PREFIXES for length in range(1, len(prefix) + 1)}
MCP_TEST_PREFIX_BYTES = max(map(len, MCP_TEST_PREFIXES))
MCP_TEST_DIGITS = b"0123456789"
DISPLAY_SKIP_REASON = "No macOS display is available"
# OpenUIIntelligentUI's display tests also need frames: SwiftUI animation and
# NSScrollView's continuous scrolling only advance on display-link ticks.
ASLEEP_SKIP_REASON = "display asleep, no frames"
MENU_TESTS = (
    "pickingFromItsMenuWritesItsBinding()", "pickingInAFormWritesItsField()",
    "aSelectsRulesAreChecked()", "selectingTheAlreadySelectedValueDoesNotApplyItsTransform()",
    "everyOptionOfALongSelectIsInItsMenuAndTheLastCanBePicked()",
)
FRAME_TESTS = (
    "markdownTableSelectedTextCanBeCopied()",
    "codeCopyWritesToTheSystemPasteboard()",
    "codeHorizontalSwipeScrollsOnlyTheBody()",
    "codeWheelScrollThroughTheWindowScrollsOnlyTheBody()",
    "horizontalSwipeMovesTheCarouselToAnotherItem(attempt:)",
    "animatedArrowClicksLandOnItemStartsAndQuickClicksAdvanceOneItemEach()",
    "arrowClicksDuringAnAnimationWorkFromTheCurrentOffset()",
    "popoverOpensAtItsTriggerAndDismissesOnKeysOutsideClickAndScroll()",
    "inlineScriptRunsAndRequestsOutsidePolicyAreBlocked()",
    "storageIsUnavailableInTwoViewsAndAFreshSession()",
    "reportedHeightFollowsContentAndClampsBothEnds()",
    "fixedHeightClampsAndIgnoresPageReports()",
    "streamingShowsPlaceholderAtInitialHeightUntilCompletion()",
    "anOverflowingPageControlOnlyReceivesClicksInsideTheViewport()",
    "navigationAndNewWindowsStayInTheAdmittedPage()",
    "onlyFiniteHeightFromThePageReachesSwift()",
    "httpCacheIsSeparateAcrossViewsAndAppProcesses()",
    "fixedPageThenFixedEmptyPageReturnsToInitialAutoHeight()",
    "spinningPageKeepsHostResponsiveReleasesViewAndExitsWhenHostExits()",
    "allocatingPageLeavesHostResponsiveAndExitsWhenDismissed()",
    "terminatedPageStaysStoppedUntilExplicitReload()",
    "largeDocumentPreparationKeepsHostControlResponsive()",
)
DISPLAY_TESTS = MENU_TESTS + FRAME_TESTS
# The reduced halves of the Carousel motion tests. They pin Reduce Motion on
# through the library's internal seam and need no frames, so they run on every
# machine, GitHub's included, and never skip. Their animated halves are in
# FRAME_TESTS.
REDUCED_MOTION_TESTS = (
    "reducedMotionArrowClicksLandOnItemStartsAndQuickClicksAdvanceOneItemEach()",
    "reducedMotionArrowClicksMoveOneItemFromWhereTheClipRests()",
)
# Matches DisplayFrames in OpenUIIntelligentUITests/CollectionWindow.swift: a
# screen whose main display is asleep, or whose display link gives no tick in
# 150 ms, has no frames.
DISPLAY_PROBE = """
import AppKit
final class Counter: NSObject { var ticks = 0; @objc func tick(_ link: CADisplayLink) { ticks += 1 } }
MainActor.assumeIsolated {
    guard let screen = NSScreen.screens.first else { print("absent"); return }
    if CGDisplayIsAsleep(CGMainDisplayID()) != 0 { print("asleep"); return }
    let counter = Counter()
    let link = screen.displayLink(target: counter, selector: #selector(Counter.tick(_:)))
    let mode = RunLoop.Mode("org.openui.tests.display-frames")
    link.add(to: .main, forMode: mode)
    let deadline = Date(timeIntervalSinceNow: 0.15)
    while Date() < deadline, counter.ticks == 0 { RunLoop.main.run(mode: mode, before: deadline) }
    link.invalidate()
    print(counter.ticks == 0 ? "asleep" : "present")
}
"""
# OpenUINative runs one `swift test` per test target, each filtered to its
# target. OpenUIIntelligentUITests runs serially: its suites share one main
# actor, so in parallel a test's reported time is mostly queueing behind other
# tests (runs 46073653498 and 46073658039: parallel 111.8 s with bodies up to
# 52.8 s, serial 128.7 s with none over 11.2 s). Every test target the package
# declares must be listed here; the audit proves each declared test ran
# exactly once, in its own target's invocation.
NATIVE_RUNS = (
    ("OpenUINativeTests", ()),
    ("OpenUIIntelligentUITests", ("--no-parallel",)),
)
DISCOVERY_TIMEOUT = 15 * 60
DISPLAY_PROBE_TIMEOUT = 30
# Each `swift test` invocation's own deadline. While it equals GATE_TIMEOUT,
# the gate deadline, which starts earlier, always fires first.
RUN_TIMEOUT = 10 * 60
# The whole gate, discovery, every invocation and cleanup included: one budget
# for the package, not one per invocation. It is kept apart from RUN_TIMEOUT
# so a test can shorten one without the other.
GATE_TIMEOUT = RUN_TIMEOUT
CLEANUP_RESERVE = 1


class IncompleteRun(ValueError):
    pass


class MCPTokenCheck:
    found = False

    def replace(self, match):
        self.found = True
        return b"[redacted MCP test token]"

    def inspect_event(self, record):
        # Re-encode decoded strings so JSON escapes cannot hide ASCII tokens.
        if MCP_TEST_TOKEN.search(json.dumps(record).encode()):
            self.found = True


class MCPTokenStream:
    def __init__(self, check):
        self.check = check
        self.prefix = b""
        self.in_digits = False

    def feed(self, chunk, *, final=False):
        if self.in_digits:
            chunk = chunk.lstrip(MCP_TEST_DIGITS)
            if not chunk:
                return b""
            self.in_digits = False
        chunk = self.prefix + chunk
        self.prefix = b""
        if not final:
            # Hold only a possible prefix. Once the first digit matches, emit
            # the marker immediately and discard any further digits in place.
            for length in range(min(len(chunk), MCP_TEST_PREFIX_BYTES), 0, -1):
                if chunk[-length:] in MCP_TEST_PARTIALS:
                    self.prefix = chunk[-length:]
                    chunk = chunk[:-length]
                    break

        def replace(match):
            self.in_digits = match.end() == len(chunk) and not self.prefix
            return self.check.replace(match)

        return MCP_TEST_TOKEN.sub(replace, chunk)


class GateTimeout(Exception):
    pass


def stop_swift(process):
    if sys.platform == "darwin":
        # SwiftPM gives its test helper a separate process group in the same
        # session. Find it even if SwiftPM has already exited.
        list_pids = ctypes.CDLL("/usr/lib/libproc.dylib").proc_listallpids
        pids = (ctypes.c_int * (2 * list_pids(None, 0) + 128))()
        for pid in pids[:list_pids(pids, ctypes.sizeof(pids))]:
            try:
                if pid > 0 and pid != process.pid and os.getsid(pid) == process.pid:
                    os.kill(pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        # The leader may have already exited and left an adopted helper.
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass


def write_output(fd, data, deadline=None):
    """Forward output within the run deadline, or best effort for gate messages."""
    blocking = os.get_blocking(fd)
    os.set_blocking(fd, False)
    try:
        pending = memoryview(data)
        while pending:
            if deadline is not None and time.monotonic() >= deadline:
                raise TimeoutError
            try:
                pending = pending[os.write(fd, pending):]
            except BlockingIOError:
                if deadline is None:
                    return
                remaining = max(0, deadline - time.monotonic())
                if not select.select([], [fd], [], remaining)[1]:
                    raise TimeoutError
    finally:
        os.set_blocking(fd, blocking)


def report(message, file, token_check=None):
    data = (message + "\n").encode()
    if token_check:
        data = MCP_TEST_TOKEN.sub(token_check.replace, data)
    write_output(file.fileno(), data)


def run_swift(command, waiting_for, timeout, *, stream=False, token_check=None):
    stderr = subprocess.STDOUT if stream else subprocess.PIPE if token_check else None
    process = subprocess.Popen(command, stdout=subprocess.PIPE,
                               stderr=stderr, start_new_session=True)
    deadline = time.monotonic() + timeout
    chunks = []
    tail = b""
    try:
        with process.stdout as output, process.stderr or nullcontext() as errors, selectors.DefaultSelector() as selector:
            selector.register(output, selectors.EVENT_READ, sys.stdout.fileno() if stream else None)
            if errors is not None:
                selector.register(errors, selectors.EVENT_READ, sys.stderr.fileno())
            redactors = {fd: MCPTokenStream(token_check) for fd in selector.get_map()} if token_check else None
            while selector.get_map():
                remaining = deadline - time.monotonic()
                ready = selector.select(remaining) if remaining > 0 else []
                if not ready:
                    raise subprocess.TimeoutExpired(command, timeout)
                for key, _ in ready:
                    chunk = os.read(key.fd, 65536)
                    if not chunk:
                        selector.unregister(key.fd)
                    if redactors:
                        chunk = redactors[key.fd].feed(chunk, final=not chunk)
                    if not chunk:
                        continue
                    if key.data is None:
                        chunks.append(chunk)
                    else:
                        if stream:
                            tail = (tail + chunk)[-4096:]
                        write_output(key.data, chunk, deadline)
        status = process.wait(timeout=max(0, deadline - time.monotonic()))
    except (subprocess.TimeoutExpired, TimeoutError):
        stop_swift(process)
        lines = tail.decode(errors="replace").splitlines()
        detail = f"; last output: {lines[-1][-500:]}" if lines else ""
        raise IncompleteRun(f"timed out after {timeout:g}s waiting for {waiting_for}{detail}") from None
    except BaseException:
        stop_swift(process)
        raise
    return status, b"".join(chunks).decode()


def declared_tests(output):
    tests = [line.strip() for line in output.splitlines() if line.strip()]
    if not tests or any(not re.fullmatch(r"\w+\.[^\s]+\([^\n]*\)", name) for name in tests):
        raise IncompleteRun("swift test list did not produce a nonempty test inventory")
    if len(set(tests)) != len(tests):
        raise IncompleteRun("swift test list returned duplicate test IDs")
    return tests


def runner_names(tests):
    # The display policy names functions even in nested suites.
    return collections.Counter(test.split(".", 1)[1].rsplit("/", 1)[-1] for test in tests)


def expected_skips(package, tests):
    if Path(package).resolve() != NATIVE_PACKAGE:
        return {}
    names = runner_names(tests)
    for name in DISPLAY_TESTS + REDUCED_MOTION_TESTS:
        if names[name] != 1:
            raise IncompleteRun(f"expected {name} exactly once in the test inventory, found {names[name]}")
    if os.environ.get("GITHUB_ACTIONS") == "true":
        return {}
    status, output = run_swift(["swift", "-e", DISPLAY_PROBE], "the macOS display probe", DISPLAY_PROBE_TIMEOUT)
    screen = output.strip()
    if status or screen not in {"absent", "asleep", "present"}:
        raise IncompleteRun("could not determine whether a macOS display is available")
    report(f"Swift test gate: macOS display {screen}", sys.stdout)
    if screen == "absent":
        return {name: DISPLAY_SKIP_REASON for name in DISPLAY_TESTS}
    if screen == "asleep":
        return {name: ASLEEP_SKIP_REASON for name in FRAME_TESTS}
    return {}


def test_target(test):
    return test.split(".", 1)[0]


def test_runs(package, tests):
    """Split the inventory into invocations: a list of (target, options, tests).

    Every declared test lands in exactly one invocation. target is None for a
    package that runs in one unfiltered invocation.
    """
    if Path(package).resolve() != NATIVE_PACKAGE:
        return [(None, (), tests)]
    planned = [target for target, _ in NATIVE_RUNS]
    unplanned = sorted({test_target(test) for test in tests} - set(planned))
    if unplanned:
        raise IncompleteRun(f"no invocation runs the test targets {unplanned}; add them to NATIVE_RUNS")
    runs = []
    for target, options in NATIVE_RUNS:
        subset = [test for test in tests if test_target(test) == target]
        if not subset:
            raise IncompleteRun(f"{target} declares no tests")
        # Swift Testing matches --filter against the test ID, "Target.Suite/test()".
        runs.append((target, ("--filter", re.escape(target) + r"\.", *options), subset))
    if collections.Counter(test for _, _, subset in runs for test in subset) != collections.Counter(tests):
        raise IncompleteRun("the invocations do not cover each declared test exactly once")
    return runs


def audit(tests, events, required_skips, token_check=None, filtered_out_targets=0):
    """Check completed tests, named skips and final summaries from Swift Testing.

    required_skips maps each test that must skip to its skip reason.
    filtered_out_targets is the number of the package's test targets that a
    --filter run excludes. SwiftPM still starts each of them, and each writes
    a passing zero-test summary to the event stream, though not the console
    (run 46083290042, reproduced with Xcode 27.2). Exactly that many such
    summaries are set aside; anything else counts as a summary of this run.
    """
    expected = collections.Counter(tests)
    finished = collections.Counter()
    summaries = []
    problems = []
    required = collections.Counter(required_skips.keys())
    skipped = collections.Counter()
    for line in events:
        record = json.loads(line)
        if token_check:
            token_check.inspect_event(record)
        if record["kind"] != "event":
            continue
        event = record["payload"]
        kind = event["kind"]
        if kind == "runEnded":
            for message in event["messages"]:
                summary = re.match(r"Test run with (\d+) tests?(?: in \d+ suites?)? (passed|failed)\b",
                                   message["text"])
                if summary:
                    summaries.append(summary.groups())
        elif kind in {"testEnded", "testSkipped"}:
            # Function IDs have a source-location suffix after the listed ID.
            # Suites also emit these events, with IDs that lack a signature.
            test = event["testID"].rsplit("/", 1)[0]
            if not test.endswith(")"):
                continue
            finished[test] += 1
            if kind == "testEnded":
                continue
            name = test.split(".", 1)[1].rsplit("/", 1)[-1]
            skipped[name] += 1
            reason = required_skips.get(name)
            skip_text = f'Test {name} skipped: "{reason}"'
            if name in required and not any(message["text"] == skip_text for message in event["messages"]):
                problems.append(f'{name} must skip with reason "{reason}"')
    if skipped != required:
        problems.append(f"skips: missing={list((required - skipped).elements())}, "
                        f"unexpected={list((skipped - required).elements())}")
    if finished != expected:
        missing = list((expected - finished).elements())
        extra = list((finished - expected).elements())
        problems.append(f"finished {sum(finished.values())}/{len(tests)} declared tests; missing={missing}, unexpected={extra}")
    if filtered_out_targets:
        empty = [summary for summary in summaries if summary == ("0", "passed")]
        if len(empty) != filtered_out_targets:
            problems.append(f"expected {filtered_out_targets} passing zero-test summaries from filtered-out test "
                            f"targets, found {len(empty)}")
        for summary in empty[:filtered_out_targets]:
            summaries.remove(summary)
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
    # The gate filters OpenUINative internally (NATIVE_RUNS); callers cannot,
    # and cannot change which invocations run in parallel.
    forbidden = {"--filter", "--skip", "--specifier", "-s", "--list-tests", "-l",
                 "--maximum-repetitions", "--repeat-until", "--disable-swift-testing",
                 "--event-stream-output-path", "--package-path", "--debugger", "list", "last",
                 "--parallel", "--no-parallel", "--num-workers"}
    if any(option.split("=", 1)[0] in forbidden for option in options):
        print("Swift test gate: filtering, repetition, parallelism and alternate test commands are not allowed",
              file=sys.stderr)
        return 2
    token_check = MCPTokenCheck() if Path(package).resolve() == CHAT_PACKAGE else None
    result = check_package(package, options, token_check)
    if token_check and token_check.found:
        report("Swift test gate: synthetic MCP credentials reached test output", sys.stderr)
        return 1
    return result


def check_package(package, options, token_check):
    command = ["swift", "test", "--package-path", package, *options]
    deadline = time.monotonic() + GATE_TIMEOUT
    # run_gate records what it is running, so the deadline message can say.
    running = {}

    def exit_at_deadline(_signum, _frame):
        try:
            report("Swift test gate: INCOMPLETE: total gate deadline reached during cleanup", sys.stderr, token_check)
        finally:
            os._exit(1)

    def stop_at_deadline(_signum, _frame):
        # Leave the hard deadline armed while exception unwinding stops Swift
        # and removes the event directory, even if that cleanup blocks.
        signal.signal(signal.SIGALRM, exit_at_deadline)
        signal.setitimer(signal.ITIMER_REAL, max(1e-6, deadline - time.monotonic()))
        raise GateTimeout

    handler = signal.signal(signal.SIGALRM, stop_at_deadline)
    reserve = min(CLEANUP_RESERVE, GATE_TIMEOUT / 10)
    signal.setitimer(signal.ITIMER_REAL, max(1e-6, deadline - reserve - time.monotonic()))
    try:
        return run_gate(package, command, token_check, running)
    except GateTimeout:
        during = (f" while running {running['label']} for {time.monotonic() - running['started']:.0f}s"
                  if running else "")
        report(f"Swift test gate: INCOMPLETE: total gate deadline reached{during} "
               f"({GATE_TIMEOUT:g}s shared by the whole package)", sys.stderr, token_check)
        return 1
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, handler)


def run_gate(package, command, token_check, running=None):
    if running is None:
        running = {}
    try:
        running.update(label=f"{package} test discovery", started=time.monotonic())
        status, output = run_swift([*command, "list"], f"{package} test discovery", DISCOVERY_TIMEOUT,
                                   token_check=token_check)
        if status:
            return status
        tests = declared_tests(output)
        required_skips = expected_skips(package, tests)
        runs = test_runs(package, tests)
    except IncompleteRun as error:
        report(f"Swift test gate: {error}", sys.stderr, token_check)
        return 1
    report(f"Swift test gate: {package} declares {len(tests)} tests", sys.stdout, token_check)
    outcomes = []
    incomplete = False
    with tempfile.TemporaryDirectory(prefix="swift-test-gate-") as directory:
        # Every invocation runs, even after an earlier one fails, so one log
        # shows each target's result. The gate deadline bounds them together.
        for index, (target, run_options, subset) in enumerate(runs):
            label = package if target is None else f"{package} {target}"
            if target is not None:
                report(f"Swift test gate: {label}: {len(subset)} tests, {' '.join(run_options)}", sys.stdout)
            names = runner_names(subset)
            run_skips = {name: reason for name, reason in required_skips.items() if names[name]}
            # SwiftPM can lose console reads split inside a UTF-8 character. Swift
            # Testing writes this invocation's records directly to its own file.
            event_path = Path(directory) / f"events-{index}.jsonl"
            running.update(label=label, started=time.monotonic())
            try:
                status, _ = run_swift([*command, *run_options, "--event-stream-output-path", str(event_path)],
                                      f"{label} test summaries", RUN_TIMEOUT, stream=True, token_check=token_check)
            except IncompleteRun as error:
                report(f"Swift test gate: INCOMPLETE: {error}", sys.stderr, token_check)
                incomplete = True
                continue
            try:
                with event_path.open(encoding="utf-8") as events:
                    outcomes.append((target, status, audit(subset, events, run_skips, token_check,
                                                                 0 if target is None else len(runs) - 1)))
            except (OSError, UnicodeError, json.JSONDecodeError) as error:
                report(f"Swift test gate: INCOMPLETE: could not read Swift Testing events for {label}: {error} "
                       f"(runner exit {status})", sys.stderr, token_check)
                incomplete = True
            except IncompleteRun as error:
                where = "" if target is None else f"{target}: "
                report(f"Swift test gate: INCOMPLETE: {where}{error} (runner exit {status})", sys.stderr, token_check)
                incomplete = True
        running.clear()
    if incomplete:
        return 1
    status = next((status for _, status, _ in outcomes if status), 0)
    result = "passed" if all(result == "passed" for _, _, result in outcomes) else "failed"
    failing = [f"{target}: summary {result}, runner exit {status}" for target, status, result in outcomes
               if target is not None and (status or result != "passed")]
    detail = f" ({'; '.join(failing)})" if failing else ""
    report(f"Swift test gate: complete {len(tests)}/{len(tests)}; summary {result}; runner exit {status}{detail}",
           sys.stdout)
    return 0 if status == 0 and result == "passed" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
