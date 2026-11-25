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
NATIVE_LIMITS = NATIVE_PACKAGE / "Tests/SwiftTestSupport/Limits.json"
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
DISPLAY_PROBE_TIMEOUT = 30
# The slowest measured full warnings-as-errors gate took 117.57s from a clean build,
# including discovery.
# 600s gives over 5x headroom for the full gate, including capture and cleanup.
TOTAL_TIMEOUT = 10 * 60
SILENCE_TIMEOUT = 195  # Standalone packages outside OpenUINative.
SAMPLE_TIMEOUT = 6.5
CAPTURE_TIMEOUT = 7
KILL_TIMEOUT = 5
EVENT_SNAPSHOT_BYTES = 256 * 1024
TEST_ID = re.compile(r"[A-Za-z_][\w.]*(?:/[\w.]+)*\([\w:]*\)")
CHECKPOINT = re.compile(rb"[0-9A-F]{8}-(?:[0-9A-F]{4}-){3}[0-9A-F]{12}")
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


def stop_swift(process, deadline=None):
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
        process.wait(timeout=remaining_time(deadline, KILL_TIMEOUT))
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


def remaining_time(deadline, maximum):
    return maximum if deadline is None else max(0, min(maximum, deadline - time.monotonic()))


def event_test_id(event):
    identifier = event.get("testID")
    if isinstance(identifier, str) and len(identifier) <= 512:
        name = identifier.rsplit("/", 1)[0]
        if TEST_ID.fullmatch(name):
            return name
    return None


class TestProgress:
    def __init__(self, event_path):
        self.event_path = event_path
        self.offset = 0
        self.partial = b""
        self.discard_line = False
        self.active = {}
        self.completed = set()
        self.case_finished_at = {}
        self.checkpoint = None

    def events(self):
        # Read one bounded chunk per poll, carrying an unfinished line across
        # reads. Keep previously seen test starts even after unrelated output.
        try:
            with os.fdopen(os.open(self.event_path, os.O_RDONLY | os.O_NONBLOCK), "rb") as events:
                info = os.fstat(events.fileno())
                if not stat.S_ISREG(info.st_mode):
                    return
                if info.st_size < self.offset:
                    self.offset, self.partial, self.discard_line = 0, b"", False
                events.seek(self.offset)
                chunk = events.read(EVENT_SNAPSHOT_BYTES)
        except OSError:
            return
        self.offset += len(chunk)
        if self.discard_line:
            _, newline, chunk = chunk.partition(b"\n")
            if not newline:
                return
            self.discard_line = False
        lines = (self.partial + chunk).split(b"\n")
        self.partial = lines.pop()
        if len(self.partial) >= EVENT_SNAPSHOT_BYTES:
            self.partial, self.discard_line = b"", True
        for line in lines:
            try:
                record = json.loads(line)
            except (ValueError, RecursionError):
                continue
            if isinstance(record, dict) and record.get("kind") == "event":
                event = record.get("payload")
                if isinstance(event, dict):
                    yield event

    def update(self):
        progressed = False
        for event in self.events():
            test = event_test_id(event)
            if not test:
                continue
            kind = event.get("kind")
            if kind == "testStarted":
                self.active[test] = None
            elif kind == "testCaseEnded":
                instant = event.get("instant")
                stamp = instant.get("absolute") if isinstance(instant, dict) else None
                if type(stamp) in (int, float) and self.case_finished_at.get(test, -math.inf) < stamp < math.inf:
                    self.case_finished_at[test] = stamp
                    progressed = True
            elif kind in ("testEnded", "testSkipped"):
                self.active.pop(test, None)
                if kind == "testEnded" and test not in self.completed:
                    self.completed.add(test)
                    progressed = True
        try:
            path = self.event_path.parent / "progress"
            with os.fdopen(os.open(path, os.O_RDONLY | os.O_NONBLOCK), "rb") as progress:
                checkpoint = progress.read(37) if stat.S_ISREG(os.fstat(progress.fileno()).st_mode) else b""
        except OSError:
            checkpoint = b""
        if CHECKPOINT.fullmatch(checkpoint) and checkpoint != self.checkpoint:
            self.checkpoint = checkpoint
            progressed = True
        return progressed

    def unfinished_tests(self):
        self.update()
        return ", ".join(self.active)


def watchdog_active(requests, session):
    # A process can keep its capture handler after its last watched case.
    # Use the shorter grace again while it is between watchdog scopes.
    for path in requests.glob("active-*"):
        try:
            if os.getsid(int(path.name.removeprefix("active-"))) == session:
                return True
        except (ProcessLookupError, ValueError, PermissionError):
            pass
    return False


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


def capture_before_kill(process, directory, requests, *, timeout=CAPTURE_TIMEOUT):
    """The sampler and all diagnostic filesystem I/O have a separate deadline."""
    command = [sys.executable, str(Path(__file__).resolve()), "--capture-swift-stacks",
               str(process.pid), str(directory), str(requests)]
    deadline = time.monotonic() + timeout
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


def show_gate_captures(paths, deadline=None):
    deadline = time.monotonic() + remaining_time(deadline, 5)
    for path in paths:
        report(f"Swift test gate: capture in {path}", sys.stderr)
        with path.open("rb") as capture:
            write_output(sys.stderr.fileno(), capture.read(2 * 1024 * 1024), deadline)


class InterruptedRun(Exception):
    pass


def interrupted(signum, _frame):
    raise InterruptedRun(signal.Signals(signum).name)


def run_swift(command, waiting_for, timeout, *, stream=False, event_path=None,
              stack_directory=None, silence_timeout=SILENCE_TIMEOUT, repetition_path=None,
              request_directory=None, startup_timeout=None, gate_deadline=None):
    environment = os.environ.copy()
    requests = event_path.parent if event_path is not None else request_directory
    if repetition_path is not None:
        environment["OPENUI_SWIFT_TEST_REPRO_EVENTS"] = str(repetition_path.resolve())
    if requests is not None:
        environment["OPENUI_SWIFT_TEST_CAPTURE_REQUESTS"] = str(requests.resolve())
    if stack_directory is not None:
        environment[STACK_DIRECTORY_ENV] = str(stack_directory.resolve())
    started = time.monotonic()
    deadline = started + timeout
    can_capture = stack_directory is not None and requests is not None
    operation, limit = waiting_for, timeout
    if gate_deadline is not None:
        # Reserve time for diagnostics and reaping inside the total budget.
        cutoff = gate_deadline - (CAPTURE_TIMEOUT + KILL_TIMEOUT if can_capture else 0)
        if cutoff <= deadline:
            operation, limit = f"{waiting_for} (total gate deadline)", max(0, cutoff - started)
            deadline = cutoff
    if started >= deadline:
        raise IncompleteRun(f"total gate deadline reached before {waiting_for}")
    process = subprocess.Popen(command, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT if stream or event_path is not None else None,
                               start_new_session=True, env=environment)
    signals = (signal.SIGINT, signal.SIGTERM)
    handlers = {sig: signal.signal(sig, interrupted) for sig in signals}
    last_progress = started
    progress = TestProgress(event_path) if event_path is not None else None
    registered = False
    chunks = []
    tail = b""
    try:
        with selectors.DefaultSelector() as selector:
            output = process.stdout
            selector.register(output, selectors.EVENT_READ)
            while True:
                now = time.monotonic()
                if now >= deadline:
                    raise TimeoutError
                silence = silence_timeout
                if progress is not None:
                    if progress.update():
                        last_progress = now
                    if startup_timeout is not None:
                        armed = watchdog_active(requests, process.pid)
                        if armed != registered:
                            registered, last_progress = armed, now
                        if not registered:
                            silence = startup_timeout
                    if now - last_progress >= silence:
                        operation, limit = f"test progress from {waiting_for}", silence
                        raise TimeoutError
                if not selector.get_map() and process.poll() is not None:
                    break
                remaining = min(deadline - now, 0.25)
                if progress is not None:
                    remaining = min(remaining, last_progress + silence - now)
                if not selector.get_map():
                    time.sleep(max(0, remaining))
                    continue
                if not selector.select(max(0, remaining)):
                    continue
                chunk = os.read(output.fileno(), 65536)
                if not chunk:
                    selector.unregister(output)
                    continue
                tail = (tail + chunk)[-4096:]
                if stream:
                    forward_until = min(deadline, last_progress + silence) if progress is not None else deadline
                    try:
                        write_output(sys.stdout.fileno(), chunk, forward_until)
                    except TimeoutError:
                        if forward_until < deadline:
                            operation, limit = f"test progress from {waiting_for}", silence
                        raise
                else:
                    chunks.append(chunk)
        status = process.returncode
        if status:
            stop_swift(process, gate_deadline)
    except (TimeoutError, InterruptedRun) as error:
        for sig in signals:
            signal.signal(sig, signal.SIG_IGN)
        reason = (f"interrupted by {error} waiting for {waiting_for}" if isinstance(error, InterruptedRun)
                  else f"timed out after {limit:g}s waiting for {operation}")
        try:
            if can_capture:
                capture_before_kill(process, stack_directory, requests,
                                    timeout=remaining_time(gate_deadline, CAPTURE_TIMEOUT))
        except (OSError, subprocess.TimeoutExpired) as capture_error:
            report(f"Swift test gate: stack capture ended: {capture_error}", sys.stderr)
        finally:
            stop_swift(process, gate_deadline)
        running = progress.unfinished_tests() if progress is not None else ""
        if running:
            reason += f"; unfinished tests: {running}"
        # Console context stays on stderr, which retains Actions log masking.
        # Never share it with the capture worker or include it in an artifact.
        lines = tail.decode(errors="replace").splitlines()
        if lines:
            report(f"Swift test gate: last output: {lines[-1][-500:]}", sys.stderr)
        if stack_directory is not None:
            try:
                show_gate_captures(sorted(stack_directory.glob(f"gate-{process.pid}-*.txt")), gate_deadline)
            except (OSError, TimeoutError) as capture_error:
                report(f"Swift test gate: stack report ended: {capture_error}", sys.stderr)
        raise IncompleteRun(reason) from None
    except BaseException:
        stop_swift(process, gate_deadline)
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


def repetition_count(value):
    # Match Swift's Int64 parsing and the minimum of one used by Carousel.
    if value is None or not re.fullmatch(r"\+?[0-9]+", value):
        return 1
    digits = value.removeprefix("+").lstrip("0")
    if not digits or len(digits) > 19:
        return 1
    count = int(digits)
    return count if count <= (1 << 63) - 1 else 1


def native_limits():
    return json.loads(NATIVE_LIMITS.read_text(encoding="utf-8"))


def silence_budget(package, limits=None):
    if Path(package).resolve() != NATIVE_PACKAGE:
        return SILENCE_TIMEOUT
    limits = limits if limits is not None else native_limits()
    stress = os.environ.get(limits["chartStressVariable"]) == "1" or any(
        repetition_count(os.environ.get(name)) > 1 for name in limits["carouselRepetitionVariables"]
    )
    scheduling = limits["schedulingSeconds"]
    case = max(limits["stressSeconds"] if stress else limits["ordinarySeconds"],
               limits["chartPollSeconds"] + scheduling, limits["nativeSeconds"],
               60 * limits["exitParentMinutes"],
               limits["exitChildSeconds"] + limits["captureSeconds"] + scheduling)
    return case + limits["captureSeconds"] + scheduling


def expected_skips(package, tests, *, gate_deadline=None):
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
            request_directory=Path(scratch), stack_directory=stack_location(package), gate_deadline=gate_deadline,
        )
    screen = output.strip()
    if status or screen not in {"absent", "present"}:
        raise IncompleteRun("could not determine whether a macOS display is available")
    return DISPLAY_TESTS if screen == "absent" else ()


def check_gate_deadline(deadline):
    if deadline is not None and time.monotonic() >= deadline:
        raise IncompleteRun("total gate deadline reached while checking results")


def audit(tests, events, required_skips, *, allow_empty_target=False, gate_deadline=None):
    """Check completed tests, named skips and final summaries from Swift Testing."""
    expected = collections.Counter(tests)
    finished = collections.Counter()
    summaries = []
    empty_targets = 0
    problems = []
    required = collections.Counter(required_skips)
    skipped = collections.Counter()
    for line in events:
        check_gate_deadline(gate_deadline)
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


def legacy_repetition_results(output, names, repetitions, *, gate_deadline=None):
    """Swift Testing 2084 prints each repeat start, then one pass per function."""
    starts = collections.defaultdict(list)
    passed = collections.Counter()
    summaries = []
    for line in output.splitlines():
        check_gate_deadline(gate_deadline)
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


def repetition_results(events, proof, output, tests, repetitions, *, gate_deadline=None):
    ids = repro_ids(tests)
    names = set(ids.values())
    records = []
    for line in events:
        check_gate_deadline(gate_deadline)
        records.append(line)
    try:
        verdict = audit(tuple(ids), records, (), allow_empty_target=True, gate_deadline=gate_deadline)
    except IncompleteRun as error:
        raise IncompleteRun(f"repetition run did not finish: {error}") from None
    if verdict != "passed":
        raise IncompleteRun("repetition run failed")

    pairs = {}
    completed = set()
    started = collections.Counter()
    unindexed_open = {}
    unindexed_end = {}
    unindexed_counts = collections.Counter()
    for line in records:
        check_gate_deadline(gate_deadline)
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
                timestamp = event.get("instant")
                instant = timestamp.get("absolute") if isinstance(timestamp, dict) else None
                if type(instant) not in (int, float) or not -math.inf < instant < math.inf:
                    raise IncompleteRun(f"invalid unindexed repetition timestamp for {identifier}")
                if (kind == "testCaseStarted" and identifier not in unindexed_open
                        and unindexed_counts[identifier] < repetitions
                        and instant > unindexed_end.get(identifier, -math.inf)):
                    unindexed_open[identifier] = instant
                elif (kind == "testCaseEnded" and identifier in unindexed_open
                      and instant >= unindexed_open[identifier]):
                    del unindexed_open[identifier]
                    unindexed_end[identifier] = instant
                    unindexed_counts[identifier] += 1
                else:
                    raise IncompleteRun(f"duplicate or unmatched unindexed repetition for {identifier}")
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
        if unindexed_counts or unindexed_open:
            raise IncompleteRun("mixed indexed and unindexed Swift Testing repetitions")
        if pairs or completed != expected:
            raise IncompleteRun("missing Swift Testing repetition completions")
    else:
        if (unindexed_open or unindexed_counts
                and unindexed_counts != collections.Counter(dict.fromkeys(ids, repetitions))):
            raise IncompleteRun("missing unindexed Swift Testing repetition completions")
        if repetitions > 1:
            legacy_repetition_results(output, names, repetitions, gate_deadline=gate_deadline)

    counts = collections.Counter()
    slowest = collections.Counter()
    for line in proof:
        check_gate_deadline(gate_deadline)
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


def reproduce_text(iterations, options, gate_deadline):
    command = ["swift", "test", "--package-path", str(NATIVE_PACKAGE), *options, "--skip-build"]
    directory = stack_location(NATIVE_PACKAGE)
    limits = native_limits()
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
                            f"{operation} test summaries", TOTAL_TIMEOUT,
                            event_path=events, stack_directory=directory, repetition_path=proof,
                            silence_timeout=silence_budget(NATIVE_PACKAGE, limits),
                            startup_timeout=limits["startupSeconds"], gate_deadline=gate_deadline,
                        )
                        try:
                            if status:
                                raise IncompleteRun(f"runner exit {status}")
                            with events.open(encoding="utf-8") as records, proof.open(encoding="utf-8") as completions:
                                counts, slowest = repetition_results(
                                    records, completions, output, tests, batch, gate_deadline=gate_deadline)
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
                try:
                    process.wait(timeout=remaining_time(gate_deadline, KILL_TIMEOUT))
                except subprocess.TimeoutExpired:
                    pass
    check_gate_deadline(gate_deadline)
    return 0


def total_timeout(_signum, _frame):
    raise IncompleteRun("total gate deadline reached")


def main(argv):
    # The phase deadlines leave time for capture. The outer timer also covers
    # discovery parsing, audit I/O and other work between subprocesses.
    gate_deadline = time.monotonic() + TOTAL_TIMEOUT
    handler = signal.signal(signal.SIGALRM, total_timeout)
    signal.setitimer(signal.ITIMER_REAL, remaining_time(gate_deadline, TOTAL_TIMEOUT))
    try:
        return run_gate(argv, gate_deadline)
    except IncompleteRun as error:
        report(f"Swift test gate: INCOMPLETE: {error}", sys.stderr)
        return 1
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, handler)


def run_gate(argv, gate_deadline):
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
            return reproduce_text(iterations, argv[2:], gate_deadline)
        except (IncompleteRun, OSError, json.JSONDecodeError) as error:
            report(f"Swift test gate: INCOMPLETE: {error}", sys.stderr)
            return 1
    package, *options = argv
    # This is a whole-package gate. Filters/repetitions cannot weaken its inventory.
    if any(option.split("=", 1)[0] in FORBIDDEN_OPTIONS for option in options):
        print("Swift test gate: filtering, repetition and alternate test commands are not allowed", file=sys.stderr)
        return 2
    command = ["swift", "test", "--package-path", package, *options]
    with tempfile.TemporaryDirectory(prefix="swift-test-gate-") as directory:
        # SwiftPM can lose console reads split inside a UTF-8 character. Swift
        # Testing writes the records for all targets directly to this file.
        event_path = Path(directory) / "events.jsonl"
        stack_directory = stack_location(package)
        try:
            status, output = run_swift(
                [*command, "list"], f"{package} test discovery", TOTAL_TIMEOUT,
                request_directory=Path(directory), stack_directory=stack_directory, gate_deadline=gate_deadline,
            )
            if status:
                return status
            tests = declared_tests(output)
            required_skips = expected_skips(package, tests, gate_deadline=gate_deadline)
            limits = native_limits() if Path(package).resolve() == NATIVE_PACKAGE else None
            report(f"Swift test gate: {package} declares {len(tests)} tests", sys.stdout)
            status, _ = run_swift([*command, "--event-stream-output-path", str(event_path)],
                                  f"{package} test summaries", TOTAL_TIMEOUT, stream=True,
                                  event_path=event_path, stack_directory=stack_directory,
                                  silence_timeout=silence_budget(package, limits),
                                  startup_timeout=limits["startupSeconds"] if limits else None,
                                  gate_deadline=gate_deadline)
        except (IncompleteRun, OSError, json.JSONDecodeError) as error:
            report(f"Swift test gate: INCOMPLETE: {error}", sys.stderr)
            return 1
        try:
            with event_path.open(encoding="utf-8") as events:
                result = audit(tests, events, required_skips, gate_deadline=gate_deadline)
                check_gate_deadline(gate_deadline)
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
