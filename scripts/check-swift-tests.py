#!/usr/bin/env python3
"""Run a complete Swift Testing package; exit zero only with a complete passing run.

Usage: python3 scripts/check-swift-tests.py PACKAGE [SwiftPM build options...]
Expected tests are discovered with `swift test list` on every invocation.
"""
import collections
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
DISPLAY_SKIP_REASON = "No macOS display is available"
MENU_TESTS = (
    "pickingFromItsMenuWritesItsBinding()", "pickingInAFormWritesItsField()",
    "aSelectsRulesAreChecked()", "selectingTheAlreadySelectedValueDoesNotApplyItsTransform()",
    "everyOptionOfALongSelectIsInItsMenuAndTheLastCanBePicked()",
)
DISPLAY_TESTS = MENU_TESTS + (
    "markdownTableSelectedTextCanBeCopied()",
    "horizontalSwipeMovesTheCarouselToAnotherItem()",
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
DISCOVERY_TIMEOUT = 15 * 60
DISPLAY_PROBE_TIMEOUT = 30
RUN_TIMEOUT = 10 * 60
CLEANUP_RESERVE = 1


class IncompleteRun(ValueError):
    pass


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


def report(message, file):
    write_output(file.fileno(), (message + "\n").encode())


def run_swift(command, waiting_for, timeout, *, stream=False):
    process = subprocess.Popen(command, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT if stream else None,
                               start_new_session=True)
    deadline = time.monotonic() + timeout
    chunks = []
    tail = b""
    try:
        with process.stdout as output, selectors.DefaultSelector() as selector:
            selector.register(output, selectors.EVENT_READ)
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise subprocess.TimeoutExpired(command, timeout)
                chunk = os.read(output.fileno(), 65536)
                if not chunk:
                    break
                if stream:
                    tail = (tail + chunk)[-4096:]
                    write_output(sys.stdout.fileno(), chunk, deadline)
                else:
                    chunks.append(chunk)
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
        return ()
    names = runner_names(tests)
    for name in DISPLAY_TESTS:
        if names[name] != 1:
            raise IncompleteRun(f"expected {name} exactly once in the test inventory, found {names[name]}")
    if os.environ.get("GITHUB_ACTIONS") == "true":
        return ()
    status, output = run_swift(
        ["swift", "-e", 'import AppKit; print(NSScreen.screens.isEmpty ? "absent" : "present")'],
        "the macOS display probe", DISPLAY_PROBE_TIMEOUT,
    )
    screen = output.strip()
    if status or screen not in {"absent", "present"}:
        raise IncompleteRun("could not determine whether a macOS display is available")
    return DISPLAY_TESTS if screen == "absent" else ()


def audit(tests, events, required_skips):
    """Check completed tests, named skips and final summaries from Swift Testing."""
    expected = collections.Counter(tests)
    finished = collections.Counter()
    summaries = []
    problems = []
    required = collections.Counter(required_skips)
    skipped = collections.Counter()
    for line in events:
        record = json.loads(line)
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
            skip_text = f'Test {name} skipped: "{DISPLAY_SKIP_REASON}"'
            if name in required and not any(message["text"] == skip_text for message in event["messages"]):
                problems.append(f'{name} must skip with reason "{DISPLAY_SKIP_REASON}"')
    if skipped != required:
        problems.append(f"skips: missing={list((required - skipped).elements())}, "
                        f"unexpected={list((skipped - required).elements())}")
    if finished != expected:
        missing = list((expected - finished).elements())
        extra = list((finished - expected).elements())
        problems.append(f"finished {sum(finished.values())}/{len(tests)} declared tests; missing={missing}, unexpected={extra}")
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
                 "--event-stream-output-path", "--package-path", "--debugger", "list", "last"}
    if any(option.split("=", 1)[0] in forbidden for option in options):
        print("Swift test gate: filtering, repetition and alternate test commands are not allowed", file=sys.stderr)
        return 2
    command = ["swift", "test", "--package-path", package, *options]
    deadline = time.monotonic() + RUN_TIMEOUT

    def exit_at_deadline(_signum, _frame):
        try:
            report("Swift test gate: INCOMPLETE: total gate deadline reached during cleanup", sys.stderr)
        finally:
            os._exit(1)

    def stop_at_deadline(_signum, _frame):
        # Leave the hard deadline armed while exception unwinding stops Swift
        # and removes the event directory, even if that cleanup blocks.
        signal.signal(signal.SIGALRM, exit_at_deadline)
        signal.setitimer(signal.ITIMER_REAL, max(1e-6, deadline - time.monotonic()))
        raise GateTimeout

    handler = signal.signal(signal.SIGALRM, stop_at_deadline)
    reserve = min(CLEANUP_RESERVE, RUN_TIMEOUT / 10)
    signal.setitimer(signal.ITIMER_REAL, max(1e-6, deadline - reserve - time.monotonic()))
    try:
        return run_gate(package, command)
    except GateTimeout:
        report("Swift test gate: INCOMPLETE: total gate deadline reached", sys.stderr)
        return 1
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, handler)


def run_gate(package, command):
    try:
        status, output = run_swift([*command, "list"], f"{package} test discovery", DISCOVERY_TIMEOUT)
        if status:
            return status
        tests = declared_tests(output)
        required_skips = expected_skips(package, tests)
    except IncompleteRun as error:
        print(f"Swift test gate: {error}", file=sys.stderr)
        return 1
    report(f"Swift test gate: {package} declares {len(tests)} tests", sys.stdout)
    with tempfile.TemporaryDirectory(prefix="swift-test-gate-") as directory:
        # SwiftPM can lose console reads split inside a UTF-8 character. Swift
        # Testing writes the records for all targets directly to this file.
        event_path = Path(directory) / "events.jsonl"
        try:
            status, _ = run_swift([*command, "--event-stream-output-path", str(event_path)],
                                  f"{package} test summaries", RUN_TIMEOUT, stream=True)
        except IncompleteRun as error:
            report(f"Swift test gate: INCOMPLETE: {error}", sys.stderr)
            return 1
        try:
            with event_path.open(encoding="utf-8") as events:
                result = audit(tests, events, required_skips)
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            report(f"Swift test gate: INCOMPLETE: could not read Swift Testing events: {error} (runner exit {status})",
                   sys.stderr)
            return 1
        except IncompleteRun as error:
            report(f"Swift test gate: INCOMPLETE: {error} (runner exit {status})", sys.stderr)
            return 1
    report(f"Swift test gate: complete {len(tests)}/{len(tests)}; summary {result}; runner exit {status}", sys.stdout)
    return 0 if status == 0 and result == "passed" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
