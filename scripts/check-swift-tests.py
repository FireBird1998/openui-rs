#!/usr/bin/env python3
"""Run a complete Swift Testing package; exit zero only with a complete passing run.

Usage: python3 scripts/check-swift-tests.py PACKAGE [SwiftPM build options...]
       python3 scripts/check-swift-tests.py --reproduce-text ITERATIONS [SwiftPM build options...]
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
SILENCE_TIMEOUT = 120
SAMPLE_TIMEOUT = 12
STACK_DIRECTORY_ENV = "OPENUI_SWIFT_TEST_STACKS"
TEXT_REPRO_TESTS = (
    "aClampedInlineControlAndTheQuotePaddingDoNotOpenLinks",
    "inlineLinksInheritParagraphTypeAndInkWhileBadgesKeepTheirTone",
    "listBodiesAndStandaloneLinksInheritLineHeightAndRichTitlesUseMediumWeight",
)
PARALLEL_REPRO_TESTS = (
    "gridRoundsClampsAndDropsColumnsAtTheMinimumIncludingGap",
    "nestedGridDistributesStretchHeightAcrossAutoRows",
)


class IncompleteRun(ValueError):
    pass


def swift_children(session):
    if sys.platform != "darwin":
        return {}
    # SwiftPM gives its test helper a separate process group in the same
    # session. Find it even if SwiftPM has already exited.
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib")
    pids = (ctypes.c_int * (2 * libproc.proc_listallpids(None, 0) + 128))()
    children = {}
    for pid in pids[:libproc.proc_listallpids(pids, ctypes.sizeof(pids))]:
        try:
            if pid > 0 and pid != session and os.getsid(pid) == session:
                name = ctypes.create_string_buffer(256)
                libproc.proc_name(pid, name, len(name))
                children[pid] = name.value.decode(errors="replace")
        except (ProcessLookupError, PermissionError):
            pass
    return children


def stop_swift(process):
    for pid in swift_children(process.pid):
        try:
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


def active_tests(event_path):
    active = {}
    if event_path is not None and event_path.is_file():
        with event_path.open(encoding="utf-8", errors="replace") as events:
            for line in events:
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    # The runner can still be writing the last event.
                    continue
                if record.get("kind") != "event":
                    continue
                event = record.get("payload", {})
                test = event.get("testID", "").rsplit("/", 1)[0]
                if not test.endswith(")"):
                    continue
                if event.get("kind") == "testStarted":
                    active[test] = None
                elif event.get("kind") in {"testEnded", "testSkipped"}:
                    active.pop(test, None)
    return ", ".join(active)


def capture_stacks(process, directory, reason, requests=None):
    children = swift_children(process.pid)
    runners = [pid for pid, name in children.items()
               if name.startswith("swiftpm-testing") or name.endswith(("Tests", ".xctest")) or name == "xctest"]
    if requests is not None:
        runners.extend(pid for pid in children if (requests / f"ready-{pid}").is_file())
    runners = sorted(set(runners)) or [process.pid]
    directory.mkdir(parents=True, exist_ok=True)
    report(f"Swift test gate: {reason}; sampling PIDs {runners}", sys.stderr)
    deadline = time.monotonic() + SAMPLE_TIMEOUT
    captures = {}
    fallback = {}
    for pid in runners:
        path = directory / f"gate-{pid}-{time.time_ns()}.sample.txt"
        try:
            sample = subprocess.Popen(["/usr/bin/sample", str(pid), "5", "-file", str(path)],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            captures[pid] = (sample, path)
        except OSError as error:
            fallback[pid] = (path, f"could not start sample: {error}")
    files = []
    for pid, (sample, path) in captures.items():
        try:
            _, diagnostics = sample.communicate(timeout=max(0, deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            sample.kill()
            try:
                sample.wait(timeout=0.25)
            except subprocess.TimeoutExpired:
                pass
            sample.stderr.close()
            fallback[pid] = (path, f"sample timed out after {SAMPLE_TIMEOUT}s")
        else:
            if sample.returncode:
                fallback[pid] = (path, diagnostics.decode(errors="replace").strip() or
                                 f"sample exited {sample.returncode}")
            elif not path.is_file():
                fallback[pid] = (path, "sample produced no report")
        if path.is_file():
            files.append(path)
    pending = {}
    for pid, (path, failure) in fallback.items():
        diagnostic = path.with_suffix(".diagnostic.txt")
        diagnostic.write_text(f"{reason}\nPID {pid}: {failure}\n", encoding="utf-8")
        files.append(diagnostic)
        if requests is not None and (requests / f"ready-{pid}").is_file():
            try:
                os.kill(pid, signal.SIGUSR2)
            except ProcessLookupError:
                continue
            pending[pid] = path.with_suffix(".backtrace.txt")
    deadline = time.monotonic() + 6
    while pending and time.monotonic() < deadline:
        for pid, path in list(pending.items()):
            response = requests / f"stack-{pid}.txt"
            if response.is_file():
                path.write_bytes(response.read_bytes())
                files.append(path)
                del pending[pid]
        if pending:
            time.sleep(0.05)
    for pid in pending:
        report(f"Swift test gate: PID {pid} did not return in-process backtraces after 6s", sys.stderr)
    deadline = time.monotonic() + 5
    for path in files:
        report(f"Swift test gate: capture in {path}", sys.stderr)
        try:
            write_output(sys.stderr.fileno(), path.read_bytes(), deadline)
        except TimeoutError:
            break  # The workflow can still upload the full report.


def run_swift(command, waiting_for, timeout, *, stream=False, event_path=None,
              stack_directory=None, silence_timeout=SILENCE_TIMEOUT):
    environment = os.environ.copy()
    if stack_directory is not None:
        environment[STACK_DIRECTORY_ENV] = str(stack_directory.resolve())
    requests = event_path.parent if event_path is not None else None
    if requests is not None:
        environment["OPENUI_SWIFT_TEST_CAPTURE_REQUESTS"] = str(requests.resolve())
    process = subprocess.Popen(command, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT if stream else None,
                               start_new_session=True, env=environment)
    deadline = time.monotonic() + timeout
    last_activity = time.monotonic()
    event_size = 0
    chunks = []
    tail = b""
    limit = timeout
    try:
        with selectors.DefaultSelector() as selector:
            output = process.stdout
            selector.register(output, selectors.EVENT_READ)
            while True:
                now = time.monotonic()
                remaining = deadline - now
                if remaining <= 0:
                    raise subprocess.TimeoutExpired(command, timeout)
                if event_path is not None:
                    try:
                        size = event_path.stat().st_size
                    except FileNotFoundError:
                        size = 0
                    if size != event_size:
                        last_activity, event_size = now, size
                    if event_size and now - last_activity >= silence_timeout:
                        limit = silence_timeout
                        raise subprocess.TimeoutExpired(command, limit)
                    remaining = min(remaining, 1)
                if not selector.select(remaining):
                    continue
                chunk = os.read(output.fileno(), 65536)
                if not chunk:
                    break
                last_activity = time.monotonic()
                if stream:
                    tail = (tail + chunk)[-4096:]
                    write_output(sys.stdout.fileno(), chunk, deadline)
                else:
                    chunks.append(chunk)
        status = process.wait(timeout=max(0, deadline - time.monotonic()))
        if status:
            stop_swift(process)
    except (subprocess.TimeoutExpired, TimeoutError):
        lines = tail.decode(errors="replace").splitlines()
        detail = f"; last output: {lines[-1][-500:]}" if lines else ""
        operation = f"output from {waiting_for}" if limit != timeout else waiting_for
        reason = f"timed out after {limit:g}s waiting for {operation}{detail}"
        try:
            running = active_tests(event_path)
            if running:
                reason += f"; unfinished tests: {running}"
            if stack_directory is not None:
                capture_stacks(process, stack_directory, reason, requests)
        except OSError as error:
            report(f"Swift test gate: stack capture failed: {error}", sys.stderr)
        finally:
            stop_swift(process)
        raise IncompleteRun(reason) from None
    except BaseException:
        stop_swift(process)
        raise
    finally:
        process.stdout.close()
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


def repetition_results(events, tests):
    starts = {}
    counts = collections.Counter()
    slowest = collections.Counter()
    summaries = []
    for line in events:
        record = json.loads(line)
        if record["kind"] != "event":
            continue
        event = record["payload"]
        if event["kind"] == "runEnded":
            summaries.extend(message["text"] for message in event["messages"])
        test = event.get("testID", "").rsplit("/", 1)[0].rsplit("/", 1)[-1].removesuffix("()")
        if test not in tests:
            continue
        key = (test, event.get("iteration", 1))
        if event["kind"] == "testCaseStarted":
            starts[key] = event["instant"]["absolute"]
        elif event["kind"] == "testCaseEnded" and key in starts:
            elapsed = event["instant"]["absolute"] - starts.pop(key)
            slowest[test] = max(slowest[test], elapsed)
            counts[test] += 1
    passed = any(re.match(rf"Test run with {len(tests)} tests? in \d+ suites? passed\b", text)
                 for text in summaries)
    return counts, slowest, passed


def reproduce_text(iterations, options):
    command = ["swift", "test", "--package-path", str(NATIVE_PACKAGE), *options, "--skip-build"]
    directory = Path(os.environ.get(STACK_DIRECTORY_ENV, NATIVE_PACKAGE / ".build/swift-test-stacks"))
    for loaded in (False, True):
        load = []
        try:
            if loaded:
                for _ in range(4):
                    load.append(subprocess.Popen(["/usr/bin/yes"], stdout=subprocess.DEVNULL))
            for parallel in (True, False):
                mode = f"{'loaded' if loaded else 'idle'} {'parallel' if parallel else 'serial'}"
                tests = TEXT_REPRO_TESTS + (PARALLEL_REPRO_TESTS if parallel else ())
                totals = collections.Counter()
                maxima = collections.Counter()
                for completed in range(0, iterations, 10):
                    batch = min(10, iterations - completed)
                    operation = f"text reproduction ({mode}, iterations {completed + 1}-{completed + batch})"
                    with tempfile.TemporaryDirectory(prefix="swift-text-repro-") as scratch:
                        events = Path(scratch) / "events.jsonl"
                        report(f"Swift test gate: starting {operation}", sys.stdout)
                        status, output = run_swift(
                            [*command, "--filter", "|".join(tests), "--parallel" if parallel else "--no-parallel",
                             "--maximum-repetitions", str(batch), "--repeat-until", "fail",
                             "--event-stream-output-path", str(events)],
                            f"{operation} test summaries", RUN_TIMEOUT,
                            event_path=events, stack_directory=directory,
                        )
                        with events.open(encoding="utf-8") as records:
                            counts, slowest, passed = repetition_results(records, tests)
                    totals.update(counts)
                    maxima |= slowest
                    if status or not passed or counts != collections.Counter(dict.fromkeys(tests, batch)):
                        report(output, sys.stderr)
                        raise IncompleteRun(f"{operation} failed (runner exit {status}); "
                                            f"completed iterations in {mode}: {dict(totals)}")
                for test in tests:
                    report(f"Swift test gate: {mode}: {test}: {totals[test]}/{iterations} passed, "
                           f"slowest {maxima[test]:.3f}s", sys.stdout)
        finally:
            for process in load:
                process.kill()
            for process in load:
                process.wait(timeout=5)
    return 0


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    if argv[0] == "--reproduce-text":
        try:
            iterations = int(argv[1])
            if iterations <= 0:
                raise ValueError
        except (IndexError, ValueError):
            print("Swift test gate: --reproduce-text requires a positive iteration count", file=sys.stderr)
            return 2
        try:
            return reproduce_text(iterations, argv[2:])
        except (IncompleteRun, OSError, json.JSONDecodeError) as error:
            report(f"Swift test gate: INCOMPLETE: {error}", sys.stderr)
            return 1
    package, *options = argv
    # This is a whole-package gate. Filters/repetitions cannot weaken its inventory.
    forbidden = {"--filter", "--skip", "--specifier", "-s", "--list-tests", "-l",
                 "--maximum-repetitions", "--repeat-until", "--disable-swift-testing",
                 "--event-stream-output-path", "--package-path", "--debugger", "list", "last"}
    if any(option.split("=", 1)[0] in forbidden for option in options):
        print("Swift test gate: filtering, repetition and alternate test commands are not allowed", file=sys.stderr)
        return 2
    command = ["swift", "test", "--package-path", package, *options]
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
        stack_directory = Path(os.environ.get(STACK_DIRECTORY_ENV, Path(package) / ".build/swift-test-stacks"))
        try:
            status, _ = run_swift([*command, "--event-stream-output-path", str(event_path)],
                                  f"{package} test summaries", RUN_TIMEOUT, stream=True,
                                  event_path=event_path, stack_directory=stack_directory)
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
