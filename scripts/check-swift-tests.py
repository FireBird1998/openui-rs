#!/usr/bin/env python3
"""Run a complete Swift Testing package; exit zero only with a complete passing run.

Usage: python3 scripts/check-swift-tests.py PACKAGE [SwiftPM build options...]
       python3 scripts/check-swift-tests.py --reproduce-text ITERATIONS [SwiftPM build options...]
The whole-package gate discovers expected tests with `swift test list`.
"""
import collections
import ctypes
import json
import math
import os
from pathlib import Path
import re
import select
import selectors
import signal
import stat
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
SILENCE_TIMEOUT = 195
STRESS_SILENCE_TIMEOUT = 300
SAMPLE_TIMEOUT = 6.5
CAPTURE_TIMEOUT = 7
EVENT_SNAPSHOT_BYTES = 256 * 1024
TEST_ID = re.compile(r"[A-Za-z_][\w.]*(?:/[\w.]+)*\([\w:]*\)")
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

FORBIDDEN_OPTIONS = {
    "--filter", "--skip", "--specifier", "-s", "--list-tests", "-l",
    "--maximum-repetitions", "--repeat-until", "--disable-swift-testing",
    "--event-stream-output-path", "--package-path", "--debugger", "list", "last", "--",
}


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


def event_snapshot(path):
    if path is None:
        return
    try:
        with os.fdopen(os.open(path, os.O_RDONLY | os.O_NONBLOCK), "rb") as events:
            info = os.fstat(events.fileno())
            if not stat.S_ISREG(info.st_mode):
                return
            offset = max(0, info.st_size - EVENT_SNAPSHOT_BYTES)
            events.seek(offset)
            lines = events.read(EVENT_SNAPSHOT_BYTES).splitlines()
    except OSError:
        return
    for line in lines:
        try:
            record = json.loads(line)
        except (ValueError, RecursionError):
            continue
        if isinstance(record, dict) and record.get("kind") == "event":
            event = record.get("payload")
            if isinstance(event, dict):
                yield event


def event_test_id(event):
    identifier = event.get("testID")
    if isinstance(identifier, str) and len(identifier) <= 512:
        name = identifier.rsplit("/", 1)[0]
        if TEST_ID.fullmatch(name):
            return name
    return None


def active_tests(event_path):
    active = {}
    for event in event_snapshot(event_path):
        test = event_test_id(event)
        if test:
            if event.get("kind") == "testStarted":
                active[test] = None
            elif event.get("kind") in ("testEnded", "testSkipped"):
                active.pop(test, None)
    return ", ".join(active)


def latest_test_completion(event_path):
    latest = 0
    for event in event_snapshot(event_path):
        if event.get("kind") in ("testEnded", "testCaseEnded", "testSkipped") and event_test_id(event):
            instant = event.get("instant")
            if isinstance(instant, dict) and type(instant.get("absolute")) in (int, float):
                latest = max(latest, instant["absolute"])
    return latest


def capture_stacks(process_id, directory, requests):
    children = swift_children(process_id)
    candidates = {process_id, *children}
    ready = {pid for pid in candidates if (requests / f"ready-{pid}").is_file()}
    runners = ready | {pid for pid, name in children.items()
                       if name.startswith("swiftpm-testing") or name.endswith(("Tests", ".xctest")) or name == "xctest"}
    runners = runners or {process_id}
    directory.mkdir(parents=True, exist_ok=True)
    prefix = f"gate-{process_id}-{time.time_ns()}"
    # Prefer an in-process capture when available. It also names tests
    # before SwiftPM has published events.
    for pid in ready:
        try:
            os.kill(pid, signal.SIGUSR2)
        except (ProcessLookupError, PermissionError):
            pass

    def collect_responses():
        for pid in tuple(ready):
            response = requests / f"stack-{pid}.txt"
            if response.is_file():
                with response.open("rb") as stack:
                    (directory / f"{prefix}-{pid}.backtrace.txt").write_bytes(stack.read(2 * 1024 * 1024))
                ready.remove(pid)
                runners.remove(pid)

    deadline = time.monotonic() + SAMPLE_TIMEOUT
    grace = min(deadline, time.monotonic() + 0.5)
    while ready and time.monotonic() < grace:
        collect_responses()
        if ready:
            time.sleep(0.05)
    with tempfile.TemporaryDirectory(prefix="sample-", dir=requests) as scratch:
        captures = {}
        for pid in runners:
            path = Path(scratch) / f"{pid}.txt"
            try:
                sampler = subprocess.Popen(["/usr/bin/sample", str(pid), "5", "-file", str(path)],
                                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                captures[pid] = (sampler, path)
            except OSError:
                pass
        for pid, (sampler, path) in captures.items():
            try:
                status = sampler.wait(timeout=max(0, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                sampler.kill()
                try:
                    sampler.wait(timeout=0.05)
                except subprocess.TimeoutExpired:
                    pass
                continue
            if status == 0 and path.is_file():
                # sample also writes process metadata and binary image paths.
                # Keep its call graph, never sampler or test console output.
                with path.open("rb") as report_file:
                    graph = report_file.read(2 * 1024 * 1024).partition(b"Call graph:\n")[2]
                graph = graph.partition(b"Total number in stack")[0].partition(b"Binary Images:")[0]
                if graph:
                    (directory / f"{prefix}-{pid}.sample.txt").write_bytes(b"Call graph:\n" + graph)
        collect_responses()


def capture_before_kill(process, directory, requests):
    """The sampler and all diagnostic filesystem I/O have a separate deadline."""
    command = [sys.executable, str(Path(__file__).resolve()), "--capture-swift-stacks",
               str(process.pid), str(directory), str(requests)]
    deadline = time.monotonic() + CAPTURE_TIMEOUT
    worker = subprocess.Popen(command, start_new_session=True,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        worker.wait(timeout=max(0, deadline - time.monotonic()))
    finally:
        # This also terminates a sampler whose worker exited without reaping it.
        try:
            os.killpg(worker.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def show_gate_captures(paths):
    deadline = time.monotonic() + 5
    for path in paths:
        report(f"Swift test gate: capture in {path}", sys.stderr)
        with path.open("rb") as capture:
            write_output(sys.stderr.fileno(), capture.read(2 * 1024 * 1024), deadline)


class InterruptedRun(Exception):
    pass


def interrupted(signum, _frame):
    raise InterruptedRun(signal.Signals(signum).name)


def run_swift(command, waiting_for, timeout, *, stream=False, event_path=None,
              stack_directory=None, silence_timeout=SILENCE_TIMEOUT, repetition_path=None, request_directory=None):
    environment = os.environ.copy()
    requests = event_path.parent if event_path is not None else request_directory
    if repetition_path is not None:
        environment["OPENUI_SWIFT_TEST_REPRO_EVENTS"] = str(repetition_path.resolve())
    if requests is not None:
        environment["OPENUI_SWIFT_TEST_CAPTURE_REQUESTS"] = str(requests.resolve())
    if stack_directory is not None:
        environment[STACK_DIRECTORY_ENV] = str(stack_directory.resolve())
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT if stream or event_path is not None else None,
                               start_new_session=True, env=environment)
    signals = (signal.SIGINT, signal.SIGTERM)
    handlers = {sig: signal.signal(sig, interrupted) for sig in signals}
    deadline = started + timeout
    last_activity = last_progress = started
    event_size = progress_stamp = last_completion = 0
    progress_at_renewal = started
    chunks = []
    tail = b""
    operation, limit = waiting_for, timeout
    try:
        with selectors.DefaultSelector() as selector:
            output = process.stdout
            selector.register(output, selectors.EVENT_READ)
            while True:
                now = time.monotonic()
                if event_path is not None:
                    try:
                        size = event_path.stat().st_size
                    except FileNotFoundError:
                        size = 0
                    if size != event_size:
                        last_activity, event_size = now, size
                        completion = latest_test_completion(event_path)
                        if completion > last_completion:
                            last_completion, last_progress = completion, now
                    try:
                        stamp = (requests / "progress").stat().st_mtime_ns
                    except FileNotFoundError:
                        stamp = 0
                    if stamp > progress_stamp:
                        progress_stamp, last_activity, last_progress = stamp, now, now
                    if now - last_activity >= silence_timeout:
                        operation, limit = f"output from {waiting_for}", silence_timeout
                        raise TimeoutError
                if now >= deadline:
                    if last_progress > progress_at_renewal and now - last_progress < silence_timeout:
                        progress_at_renewal = last_progress
                        deadline = now + timeout
                        report(f"Swift test gate: continuing {waiting_for} after recent test progress", sys.stderr)
                    else:
                        raise TimeoutError
                if not selector.get_map() and process.poll() is not None:
                    break
                remaining = min(deadline - now, 0.25)
                if event_path is not None:
                    remaining = min(remaining, last_activity + silence_timeout - now)
                if not selector.get_map():
                    time.sleep(max(0, remaining))
                    continue
                if not selector.select(max(0, remaining)):
                    continue
                chunk = os.read(output.fileno(), 65536)
                if not chunk:
                    selector.unregister(output)
                    continue
                last_activity = time.monotonic()
                tail = (tail + chunk)[-4096:]
                if stream:
                    forward_until = min(deadline, last_activity + silence_timeout) if requests else deadline
                    try:
                        write_output(sys.stdout.fileno(), chunk, forward_until)
                    except TimeoutError:
                        if forward_until < deadline:
                            operation, limit = f"output from {waiting_for}", silence_timeout
                        raise
                else:
                    chunks.append(chunk)
        status = process.returncode
        if status:
            stop_swift(process)
    except (TimeoutError, InterruptedRun) as error:
        for sig in signals:
            signal.signal(sig, signal.SIG_IGN)
        reason = (f"interrupted by {error} waiting for {waiting_for}" if isinstance(error, InterruptedRun)
                  else f"timed out after {limit:g}s waiting for {operation}")
        try:
            if stack_directory is not None and requests is not None:
                capture_before_kill(process, stack_directory, requests)
        except (OSError, subprocess.TimeoutExpired) as capture_error:
            report(f"Swift test gate: stack capture ended: {capture_error}", sys.stderr)
        finally:
            stop_swift(process)
        running = active_tests(event_path)
        if running:
            reason += f"; unfinished tests: {running}"
        # Console context stays on stderr, which retains Actions log masking.
        # Never share it with the capture worker or include it in an artifact.
        lines = tail.decode(errors="replace").splitlines()
        if lines:
            report(f"Swift test gate: last output: {lines[-1][-500:]}", sys.stderr)
        if stack_directory is not None:
            try:
                show_gate_captures(sorted(stack_directory.glob(f"gate-{process.pid}-*.txt")))
            except (OSError, TimeoutError) as capture_error:
                report(f"Swift test gate: stack report ended: {capture_error}", sys.stderr)
        raise IncompleteRun(reason) from None
    except BaseException:
        stop_swift(process)
        raise
    finally:
        for sig, handler in handlers.items():
            signal.signal(sig, handler)
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


def stack_location(package):
    return Path(os.environ.get(STACK_DIRECTORY_ENV, Path(package) / ".build/swift-test-stacks")).resolve()


def silence_budget(package):
    if Path(package).resolve() == NATIVE_PACKAGE:
        if os.environ.get("OPENUI_CHART_ABSOLUTE_MAX") == "1":
            return STRESS_SILENCE_TIMEOUT
        for name in ("OPENUI_REPEAT_CAROUSEL_OVERFLOW", "OPENUI_REPEAT_CAROUSEL_SWIPE"):
            value = os.environ.get(name, "")
            if re.fullmatch(r"\+?[0-9]{1,19}", value) and 1 < int(value) <= sys.maxsize:
                return STRESS_SILENCE_TIMEOUT
    return SILENCE_TIMEOUT


def expected_skips(package, tests):
    if Path(package).resolve() != NATIVE_PACKAGE:
        return ()
    names = runner_names(tests)
    for name in DISPLAY_TESTS:
        if names[name] != 1:
            raise IncompleteRun(f"expected {name} exactly once in the test inventory, found {names[name]}")
    if os.environ.get("GITHUB_ACTIONS") == "true":
        return ()
    with tempfile.TemporaryDirectory(prefix="swift-display-probe-") as scratch:
        status, output = run_swift(
            ["swift", "-e", 'import AppKit; print(NSScreen.screens.isEmpty ? "absent" : "present")'],
            "the macOS display probe", DISPLAY_PROBE_TIMEOUT,
            request_directory=Path(scratch), stack_directory=stack_location(package),
        )
    screen = output.strip()
    if status or screen not in {"absent", "present"}:
        raise IncompleteRun("could not determine whether a macOS display is available")
    return DISPLAY_TESTS if screen == "absent" else ()


def audit(tests, events, required_skips, *, allow_empty_target=False):
    """Check completed tests, named skips and final summaries from Swift Testing."""
    expected = collections.Counter(tests)
    finished = collections.Counter()
    summaries = []
    empty_targets = 0
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
                    if allow_empty_target and summary.groups() == ("0", "passed"):
                        empty_targets += 1
                    else:
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
    if empty_targets > 1:
        problems.append(f"expected at most one unselected target summary, found {empty_targets}")
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


def repro_ids(tests):
    ids = {}
    for name in tests:
        suite = "GridDrawingTests" if name in PARALLEL_REPRO_TESTS else "TextDrawingTests"
        ids[f"OpenUIIntelligentUITests.{suite}/{name}()"] = name + "()"
    return ids


def legacy_repetition_results(output, names, repetitions):
    """Swift Testing 2084 prints each repeat start, then one pass per function."""
    starts = collections.defaultdict(list)
    passed = collections.Counter()
    summaries = []
    for line in output.splitlines():
        start = re.fullmatch(r"◇ Test (\w+\(\)) started(?: \(repetition (\d+)\))?\.", line)
        if start and start[1] in names:
            starts[start[1]].append(int(start[2] or 1))
        finish = re.fullmatch(r"✔ Test (\w+\(\)) passed after [\d.]+ seconds?\.", line)
        if finish and finish[1] in names:
            if starts[finish[1]] != list(range(1, repetitions + 1)):
                raise IncompleteRun(f"incomplete legacy repetition sequence for {finish[1]}")
            passed[finish[1]] += 1
        summary = re.fullmatch(r"✔ Test run with (\d+) tests?(?: in \d+ suites?)? passed after [\d.]+ seconds?\.", line)
        if summary:
            summaries.append(int(summary[1]))
    expected_summaries = ([len(names)], [0, len(names)], [len(names), 0])
    if (passed != collections.Counter(dict.fromkeys(names, 1)) or summaries not in expected_summaries
            or any(starts[name] != list(range(1, repetitions + 1)) for name in names)):
        raise IncompleteRun("incomplete legacy Swift Testing repetition summary")
    return collections.Counter(dict.fromkeys(names, repetitions))


def repetition_results(events, proof, output, tests, repetitions):
    ids = repro_ids(tests)
    names = set(ids.values())
    records = list(events)
    try:
        verdict = audit(tuple(ids), records, (), allow_empty_target=True)
    except IncompleteRun as error:
        raise IncompleteRun(f"repetition run did not finish: {error}") from None
    if verdict != "passed":
        raise IncompleteRun("repetition run failed")

    pairs = {}
    completed = set()
    started = collections.Counter()
    for line in records:
        record = json.loads(line)
        if record["kind"] != "event":
            continue
        event = record["payload"]
        identifier = event_test_id(event)
        kind = event["kind"]
        if identifier not in ids:
            continue
        if kind == "testStarted":
            is_repeat = any(re.search(r" started \(repetition \d+\)\.$", message["text"])
                            for message in event["messages"])
            if not is_repeat:
                started[identifier] += 1
        if kind in ("testCaseStarted", "testCaseEnded"):
            iteration = event.get("iteration", event.get("repetition"))
            if iteration is None:
                # Older event streams omit iteration ordinals. The exact
                # console sequence and the body completion stream cover those.
                continue
            if type(iteration) is not int or not 1 <= iteration <= repetitions:
                raise IncompleteRun(f"invalid repetition index for {identifier}: {iteration}")
            pair = (ids[identifier], iteration)
            instant = event["instant"]["absolute"]
            if kind == "testCaseStarted" and pair not in pairs and pair not in completed:
                pairs[pair] = instant
            elif kind == "testCaseEnded" and pair in pairs and instant >= pairs[pair]:
                del pairs[pair]
                completed.add(pair)
            else:
                raise IncompleteRun(f"duplicate or unmatched repetition {pair}")
    if started != collections.Counter(dict.fromkeys(ids, 1)):
        raise IncompleteRun("missing or duplicate Swift Testing function starts")
    expected = {(name, iteration) for name in names for iteration in range(1, repetitions + 1)}
    if completed or pairs:
        if pairs or completed != expected:
            raise IncompleteRun("missing Swift Testing repetition completions")
    elif repetitions > 1:
        legacy_repetition_results(output, names, repetitions)

    counts = collections.Counter()
    slowest = collections.Counter()
    for line in proof:
        entry = json.loads(line)
        name, iteration, elapsed = entry["test"], entry["iteration"], entry["elapsed"]
        if (name not in names or type(iteration) is not int or iteration != counts[name] + 1
                or iteration > repetitions or type(elapsed) not in (int, float)
                or not math.isfinite(elapsed) or elapsed < 0):
            raise IncompleteRun("invalid or duplicate test-body repetition completion")
        counts[name] += 1
        slowest[name] = max(slowest[name], elapsed)
    if counts != collections.Counter(dict.fromkeys(names, repetitions)):
        raise IncompleteRun(f"missing test-body repetition completions: {dict(counts)}")
    return counts, slowest


def reproduce_text(iterations, options):
    command = ["swift", "test", "--package-path", str(NATIVE_PACKAGE), *options, "--skip-build"]
    directory = stack_location(NATIVE_PACKAGE)
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
                        proof = Path(scratch) / "repetitions.jsonl"
                        proof.touch()
                        report(f"Swift test gate: starting {operation}", sys.stdout)
                        pattern = "|".join(tests)
                        status, output = run_swift(
                            [*command, "--filter", pattern, "--parallel" if parallel else "--no-parallel",
                             "--maximum-repetitions", str(batch), "--repeat-until", "fail",
                             "--event-stream-output-path", str(events)],
                            f"{operation} test summaries", RUN_TIMEOUT,
                            event_path=events, stack_directory=directory, repetition_path=proof,
                        )
                        try:
                            if status:
                                raise IncompleteRun(f"runner exit {status}")
                            with events.open(encoding="utf-8") as records, proof.open(encoding="utf-8") as completions:
                                counts, slowest = repetition_results(records, completions, output, tests, batch)
                        except (IncompleteRun, OSError, ValueError, TypeError, KeyError) as error:
                            report(output, sys.stderr)
                            raise IncompleteRun(f"{operation} failed: {error}") from None
                    totals.update(counts)
                    maxima |= slowest
                for test in tests:
                    name = test + "()"
                    report(f"Swift test gate: {mode}: {name}: {totals[name]}/{iterations} passed, "
                           f"slowest {maxima[name]:.3f}s", sys.stdout)
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
        if any(option.split("=", 1)[0] in FORBIDDEN_OPTIONS for option in argv[2:]):
            print("Swift test gate: reproduction uses the native package and its fixed test selection", file=sys.stderr)
            return 2
        try:
            return reproduce_text(iterations, argv[2:])
        except (IncompleteRun, OSError, json.JSONDecodeError) as error:
            report(f"Swift test gate: INCOMPLETE: {error}", sys.stderr)
            return 1
    package, *options = argv
    # This is a whole-package gate. Filters/repetitions cannot weaken its inventory.
    if any(option.split("=", 1)[0] in FORBIDDEN_OPTIONS for option in options):
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
        stack_directory = stack_location(package)
        try:
            status, _ = run_swift([*command, "--event-stream-output-path", str(event_path)],
                                  f"{package} test summaries", RUN_TIMEOUT, stream=True,
                                  event_path=event_path, stack_directory=stack_directory,
                                  silence_timeout=silence_budget(package))
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
    if len(sys.argv) == 5 and sys.argv[1] == "--capture-swift-stacks":
        capture_stacks(int(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[4]))
    else:
        sys.exit(main(sys.argv[1:]))
