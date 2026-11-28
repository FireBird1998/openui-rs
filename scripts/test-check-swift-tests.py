#!/usr/bin/env python3
"""Exercise the Swift completeness gate, including successful-but-truncated runners."""
import json
import os
import re
import runpy
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPT = Path(__file__).with_name("check-swift-tests.py")
CHAT_PACKAGE = SCRIPT.resolve().parents[1] / "tools/chat"
TOKEN_WARNING = "synthetic MCP credentials reached test output"
REDACTION = "[redacted MCP test token]"
ONE = "Example.Parent/Child/one()"
TWO = "Example.Parent/Child/two(_:)"
THREE = "Other.Suite/three()"
TWO_TARGETS = f"{ONE}\n{TWO}\n{THREE}\n"
NATIVE_PACKAGE = SCRIPT.resolve().parents[1] / "swift/OpenUINative"
NATIVE = "OpenUINativeTests"
INTELLIGENT_UI = "OpenUIIntelligentUITests"
# Shared by the fake `swift` commands. Like SwiftPM (run 46083290042,
# reproduced with Xcode 27.2), a --filter run reports only the matching tests
# and its target's summary, and each filtered-out target still writes a
# passing zero-test summary to the event stream. A summary's fixture-only
# "target" names its target. also maps a target to extra event lines its own
# filtered invocation reports regardless of the filter. mode changes the
# filtered invocation: "count" and "result" rewrite its own summary, "empty"
# is how many zero-test summaries each filtered-out target writes (default 1)
# and "empty_result" their result. Each invocation's arguments are appended
# to invocations.jsonl.
FAKE_RUN = """
import json, re
with Path(__file__).with_name('invocations.jsonl').open('a') as invocations:
    invocations.write(json.dumps(sys.argv[1:]) + '\\n')

def run_log(log, also, mode=None):
    mode = mode or {}
    if '--filter' not in sys.argv:
        return log
    filter_ = sys.argv[sys.argv.index('--filter') + 1]
    pattern = re.compile(filter_)
    kept = []
    for line in log.splitlines(keepends=True):
        payload = json.loads(line)['payload']
        if payload['kind'] == 'runEnded' and 'target' in payload:
            if pattern.search(payload['target'] + '.'):
                if 'count' in mode:
                    line = re.sub(r'with [0-9]+ tests', f"with {mode['count']} tests", line)
                if 'result' in mode:
                    line = line.replace(' passed after', ' ' + mode['result'] + ' after')
                kept.append(line)
            else:
                text = f"Test run with 0 tests in 0 suites {mode.get('empty_result', 'passed')} after 0.001 seconds."
                empty = {'kind': 'event', 'payload': {'kind': 'runEnded', 'messages': [{'text': text}]}}
                kept += [json.dumps(empty) + '\\n'] * mode.get('empty', 1)
            continue
        key = payload['testID'] if 'testID' in payload else payload.get('target', '') + '.'
        if pattern.search(key):
            kept.append(line)
    kept += [extra for target, extra in also.items() if pattern.search(target + '.')]
    return ''.join(kept)
"""
RUN_WITH_SHORT_DEADLINE = """
import runpy
import sys

script, runner = sys.argv[1:]
gate = runpy.run_path(script)
try:
    gate['run_swift']([sys.executable, '-c', runner], 'test summaries', 2, stream=True)
except gate['IncompleteRun'] as error:
    sys.exit(str(error))
"""
RUN_GATE_WITH_GLOBALS = """
import json
import runpy
import sys

script, package, overrides, *options = sys.argv[1:]
gate = runpy.run_path(script)
gate['check_package'].__globals__.update(json.loads(overrides))
sys.exit(gate['main']([package, *options]))
"""
RUN_GATE_WITH_CONTROLLED_READS = """
import os
import runpy
import sys

script, package, read_size = sys.argv[1:]
gate = runpy.run_path(script)
original_read = os.read

def read(fd, size):
    return original_read(fd, min(size, int(read_size)) if size == 65536 else size)

os.read = read
gate['check_package'].__globals__['RUN_TIMEOUT'] = 2
sys.exit(gate['main']([package]))
"""


def event(kind, test=None, text=None):
    payload = {"kind": kind, "messages": [{"text": text}] if text else []}
    if test is not None:
        payload["testID"] = f"{test}/Fixture.swift:1:1"
    return {"kind": "event", "payload": payload}


def finished(*tests):
    return [event("testEnded", test) for test in tests]


def cancelled(test, name, reason):
    """The records of a test cancelled as it starts, naming reason. Swift
    Testing gives a cancelled case of a parameterized test no message, so the
    trait records a known issue naming the reason first (Swift 6.4, observed
    on the Carousel suite)."""
    if name.endswith(":)"):
        issue = event("issueRecorded", test)
        issue["payload"]["messages"] = [{"text": "Issue recorded"}, {"text": reason}, {"text": reason}]
        issue["payload"]["issue"] = {"isKnown": True}
        return [issue, event("testCaseCancelled", test),
                event("testEnded", test, f"Test {name} with 1 test case passed after 0.004 seconds with 1 known issue.")]
    return [event("testCancelled", test),
            event("testEnded", test, f'Test {name} was cancelled after 0.009 seconds: "{reason}"')]


def summary(count, result="passed", target=None):
    record = event("runEnded", text=f"Test run with {count} tests in 1 suite {result} after 0.1 seconds.")
    if target is not None:
        record["payload"]["target"] = target
    return record


def native_target(name):
    # Mirrors the package: the menu tests are in OpenUINativeTests, the frame
    # and reduced-motion tests in OpenUIIntelligentUITests.
    return NATIVE if name in SwiftTestGate.MENU_TESTS or name == "ordinary()" else INTELLIGENT_UI


def target_summaries(tests, result="passed"):
    counts = {}
    for test in tests:
        counts[test.split(".", 1)[0]] = counts.get(test.split(".", 1)[0], 0) + 1
    return [summary(count, result, target) for target, count in counts.items()]


def event_log(events):
    return "".join(json.dumps(record) + "\n" for record in events)


class SwiftTestGate(unittest.TestCase):
    def test_total_deadline_bounds_cleanup_after_a_blocked_audit(self):
        # No process starts between the gate's start and the blocked audit:
        # discovery and the summary run are replaced in-process, so the deadline
        # cannot fire before the audit blocks, however slow the host. A 3 s
        # deadline gives a 0.3 s cleanup reserve (GATE_TIMEOUT / 10) between
        # the soft and hard alarms, so a short stall while unwinding cannot let
        # the hard exit fire before cleanup starts.
        # Run 46011887659 failed when a subprocess-based discovery outlasted
        # the soft deadline. Elapsed time is measured from the gate's own
        # start, not from interpreter start-up.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = f"""
import os, runpy, sys, time
from pathlib import Path

script, root = sys.argv[1:]
gate = runpy.run_path(script)["main"]
globals_ = gate.__globals__
globals_["GATE_TIMEOUT"] = 3.0
tempfile = globals_["tempfile"]

def run_swift(command, waiting_for, timeout, **options):
    if "list" in command:
        return 0, {ONE!r} + "\\n"
    os.mkfifo(command[-1])  # The audit's open() then blocks with no writer.
    return 0, ""

class SlowCleanup(tempfile.TemporaryDirectory):
    def __exit__(self, *args):
        Path(root, "cleanup-started").touch()
        time.sleep(6)
        super().__exit__(*args)

globals_["run_swift"] = run_swift
tempfile.TemporaryDirectory = SlowCleanup
Path(root, "gate-started").write_text(repr(time.time()))
sys.exit(gate([root]))
"""
            env = {**os.environ, "TMPDIR": str(root)}
            gate = subprocess.run([sys.executable, "-c", fixture, str(SCRIPT), str(root)],
                                  env=env, capture_output=True, text=True, timeout=15)
            ended = time.time()
            self.assertEqual(gate.returncode, 1, gate.stderr)
            self.assertEqual(gate.stderr.strip().splitlines()[-1],
                             "Swift test gate: INCOMPLETE: total gate deadline reached during cleanup")
            self.assertNotIn("total gate deadline reached while running", gate.stderr)
            self.assertTrue((root / "cleanup-started").is_file(), gate.stderr)
            # Cleanup never finished: its directory and FIFO are still there.
            self.assertTrue(any(path.is_fifo() for path in root.glob("swift-test-gate-*/events*.jsonl")))
            # The hard exit comes 3 s after the gate starts; an uncut cleanup
            # would end at least 8.7 s after it (2.7 s soft alarm + 6 s sleep).
            self.assertLess(ended - float((root / "gate-started").read_text()), 4.5)

    def run_viewer_gate(self, incomplete, link_status=0):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            (root / "tools/viewer/Requests").mkdir(parents=True)
            for name in ["check-viewer.sh", "check-swift-tests.py"]:
                (root / "scripts" / name).write_text(SCRIPT.with_name(name).read_text())
            (root / "tools/viewer/Requests/sync-recorded-streams.py").write_text("")
            (root / "scripts/test-check-swift-tests.py").write_text("")
            (root / "scripts/check-chat-linkage.py").write_text(f"raise SystemExit({link_status})\n")
            binding = root / "scripts/viewer.sh"
            binding.write_text("#!/bin/sh\nexit 0\n")
            binding.chmod(0o755)
            swift = root / "swift"
            packages = {}
            for package in ("swift/OpenUINative", "tools/viewer", "tools/chat"):
                if package == "swift/OpenUINative":
                    tests = [f"{native_target(name)}.Suite/{name}" for name in self.NATIVE_TESTS]
                    summaries = target_summaries(tests)
                else:
                    tests = [f"Example.Suite/{name}" for name in ("one()", "two()")]
                    summaries = [summary(len(tests))]
                records = (finished(*tests) + summaries if package != incomplete
                           else [event("testStarted", tests[0])])
                packages[package] = ("\n".join(tests), event_log(records))
            swift.write_text(f"#!{sys.executable}\nfrom pathlib import Path\nimport sys\n"
                             "if 'build' in sys.argv: sys.exit(0)\n"
                             "if '-e' in sys.argv: print('present unlocked minimises'); sys.exit(0)\n"
                             f"{FAKE_RUN}\n"
                             f"packages = {packages!r}\n"
                             "inventory, log = packages[sys.argv[sys.argv.index('--package-path') + 1]]\n"
                             "if 'list' in sys.argv:\n"
                             "    print(inventory)\n"
                             "else:\n"
                             "    Path(sys.argv[sys.argv.index('--event-stream-output-path') + 1])"
                             ".write_text(run_log(log, {}))\n")
            swift.chmod(0o755)
            return subprocess.run(["bash", str(root / "scripts/check-viewer.sh")],
                                  env={**os.environ, "PATH": str(root) + os.pathsep + os.environ["PATH"]},
                                  capture_output=True, text=True)

    def test_viewer_gate_checks_viewer_after_incomplete_native_tests(self):
        result = self.run_viewer_gate("swift/OpenUINative")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INCOMPLETE", result.stderr)
        self.assertIn("tools/viewer declares 2 tests", result.stdout)
        self.assertIn("complete 2/2", result.stdout)

    def test_viewer_gate_rejects_incomplete_viewer_tests(self):
        result = self.run_viewer_gate("tools/viewer")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INCOMPLETE", result.stderr)
        self.assertIn("tools/viewer declares 2 tests", result.stdout)

    def test_viewer_gate_rejects_incomplete_chat_app_tests(self):
        result = self.run_viewer_gate("tools/chat")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INCOMPLETE", result.stderr)
        self.assertIn("tools/chat declares 2 tests", result.stdout)

    def test_viewer_gate_passes_only_when_every_package_finishes(self):
        result = self.run_viewer_gate("no-package")
        self.assertEqual(result.returncode, 0, result.stderr)
        native_count = len(self.NATIVE_TESTS)
        self.assertIn(f"complete {native_count}/{native_count}", result.stdout)
        self.assertEqual(result.stdout.count("complete 2/2"), 2)

    def test_viewer_gate_rejects_failed_chat_linkage(self):
        result = self.run_viewer_gate("no-package", link_status=1)
        self.assertEqual(result.returncode, 1)
        self.assertIn("tools/chat declares 2 tests", result.stdout)

    def test_chat_gate_rejects_and_redacts_tokens_in_successful_test_output(self):
        for stream, token in (("stdout", "synthetic-access-1"), ("stderr", "synthetic-refresh-2")):
            with self.subTest(stream=stream):
                leak = f"OAuth reply: {token}\n"
                result = self.run_gate(
                    finished(ONE, TWO) + [summary(2)], package=CHAT_PACKAGE,
                    output=leak if stream == "stdout" else "",
                    stderr=leak if stream == "stderr" else "",
                )
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(TOKEN_WARNING, result.stderr)
                self.assertNotIn(token, result.stdout + result.stderr)
                self.assertIn(f"OAuth reply: {REDACTION}\n", result.stdout)

    def test_chat_gate_redacts_tokens_across_chunk_and_utf8_boundaries(self):
        result = self.run_gate(finished(ONE, TWO) + [summary(2)], package=CHAT_PACKAGE, read_size=1,
                               output="◇ synthetic-access-123synthetic-refresh-456✔\n")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(TOKEN_WARNING, result.stderr)
        self.assertIn(f"◇ {REDACTION}{REDACTION}✔\n", result.stdout)

    def test_chat_gate_redacts_a_token_at_eof(self):
        events = finished(ONE, TWO) + [summary(2)]
        result = self.run_gate(events, package=CHAT_PACKAGE, output="OAuth reply: synthetic-access-123")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(TOKEN_WARNING, result.stderr)
        self.assertIn(f"OAuth reply: {REDACTION}Swift test gate: complete", result.stdout)

        result = self.run_gate(events, package=CHAT_PACKAGE, output="pending: synthetic-refresh-")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("pending: synthetic-refresh-Swift test gate: complete", result.stdout)

    def test_chat_gate_redacts_timeout_tail(self):
        # A raw tail would clip the prefix and leave digits that a second
        # regex pass over the diagnostic could no longer recognize.
        result = self.run_gate([], package=CHAT_PACKAGE, read_size=65536, hang=True,
                               stderr="OAuth reply: synthetic-refresh-" + "7" * 600)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("timed out after 2s waiting for", result.stderr)
        self.assertIn(f"; last output: OAuth reply: {REDACTION}", result.stderr)
        self.assertIn(TOKEN_WARNING, result.stderr)
        self.assertIn(f"OAuth reply: {REDACTION}", result.stdout)

    def test_non_chat_gate_forwards_token_like_output_unchanged(self):
        output = "◇ synthetic-access-123synthetic-refresh-456✔\n"
        result = self.run_gate(finished(ONE, TWO) + [summary(2)], read_size=1, output=output)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertIn(output, result.stdout)

    def test_chat_gate_rejects_tokens_only_in_event_messages(self):
        token = "synthetic-refresh-123"
        records = [event("issueRecorded", ONE, f"OAuth reply: {token}"), *finished(ONE, TWO), summary(2)]
        log = event_log(records).replace(token, r"synthetic-refresh-12\u0033")
        result = self.run_gate(records, package=CHAT_PACKAGE, log=log)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(TOKEN_WARNING, result.stderr)
        self.assertIn("complete 2/2", result.stdout)
        self.assertNotIn(token, result.stdout + result.stderr)

    def test_chat_gate_redacts_event_ids_in_audit_errors(self):
        token = "synthetic-access-123"
        result = self.run_gate(finished(ONE, f"Example.Suite/{token}()") + [summary(2)], package=CHAT_PACKAGE)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(f"unexpected=['Example.Suite/{REDACTION}()']", result.stderr)
        self.assertIn(TOKEN_WARNING, result.stderr)
        self.assertNotIn(token, result.stdout + result.stderr)

    def test_chat_gate_redacts_discovery_stderr(self):
        result = self.run_gate(finished(ONE, TWO) + [summary(2)], package=CHAT_PACKAGE,
                               discovery_stderr="Build log: synthetic-access-123\n")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(f"Build log: {REDACTION}\n", result.stderr)
        self.assertIn(TOKEN_WARNING, result.stderr)
        self.assertIn("complete 2/2", result.stdout)

    def run_gate(self, events, status=0, inventory=f"{ONE}\n{TWO}\n", package="example",
                 screen="present", locked=False, minimises=True, probe=None, github=False, output="", stderr="",
                 discovery_stderr="", read_size=None, hang=False, log=None, also=None, options=(), per_target=None,
                 overrides=None, probe_after=None):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            swift = directory / "swift"
            if log is None:
                log = event_log(events)
            also = {target: event_log(records) for target, records in (also or {}).items()}
            if probe is None:
                probe = (f"{screen} {'locked' if locked else 'unlocked'} {'minimises' if minimises else 'refuses'}"
                         if screen else "")
            # probe_after answers every display probe after the first, as a
            # display that changes during the run does.
            swift.write_text(f"#!{sys.executable}\nfrom pathlib import Path\nimport sys\nimport time\n"
                             "if '-e' in sys.argv:\n"
                             "    probes = Path(__file__).with_name('probes')\n"
                             "    seen = int(probes.read_text()) if probes.exists() else 0\n"
                             "    probes.write_text(str(seen + 1))\n"
                             f"    print({probe!r} if seen == 0 or {probe_after is None!r} else {probe_after!r})\n"
                             "    sys.exit(0)\n"
                             f"{FAKE_RUN}\n"
                             "if 'list' in sys.argv:\n"
                             f"    print({inventory!r}, end='')\n"
                             f"    print({discovery_stderr!r}, end='', file=sys.stderr)\n"
                             "    sys.exit(0)\n"
                             f"print({output!r}, end='', flush=True)\n"
                             f"print({stderr!r}, end='', file=sys.stderr, flush=True)\n"
                             # per_target gives one filtered invocation its own exit
                             # status, summary result or sleep.
                             "mode = {}\n"
                             "if '--filter' in sys.argv:\n"
                             "    pattern = re.compile(sys.argv[sys.argv.index('--filter') + 1])\n"
                             f"    mode = next((m for t, m in {per_target or {}!r}.items()"
                             " if pattern.search(t + '.')), {})\n"
                             f"run = run_log({log!r}, {also!r}, mode)\n"
                             "Path(sys.argv[sys.argv.index('--event-stream-output-path') + 1]).write_text(run)\n"
                             f"if {hang!r}: time.sleep(8)\n"
                             "time.sleep(mode.get('sleep', 0))\n"
                             f"sys.exit(mode.get('status', {status}))\n")
            swift.chmod(0o755)
            command = [sys.executable, str(SCRIPT), package, *options]
            if read_size is not None:
                command = [sys.executable, "-c", RUN_GATE_WITH_CONTROLLED_READS, str(SCRIPT), package, str(read_size)]
            elif overrides is not None:
                command = [sys.executable, "-c", RUN_GATE_WITH_GLOBALS, str(SCRIPT), str(package),
                           json.dumps(overrides), *options]
            result = subprocess.run(command,
                                    env={**os.environ, "GITHUB_ACTIONS": str(github).lower(),
                                         "PATH": str(directory) + os.pathsep + os.environ["PATH"]},
                                    capture_output=True, text=True, timeout=6)
            invocations = directory / "invocations.jsonl"
            result.invocations = ([json.loads(line) for line in invocations.read_text().splitlines()]
                                  if invocations.exists() else [])
            return result

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
    REDUCED_MOTION_TESTS = (
        "reducedMotionArrowClicksLandOnItemStartsAndQuickClicksAdvanceOneItemEach()",
        "reducedMotionArrowClicksMoveOneItemFromWhereTheClipRests()",
    )
    MINIMISE_TESTS = ("aPhotoLoadedWhileItsWindowIsMinimisedDrawsWhenItShows()",)
    NATIVE_TESTS = DISPLAY_TESTS + REDUCED_MOTION_TESTS + MINIMISE_TESTS
    ASLEEP = "display asleep, no frames"
    REFUSED = "AppKit refused to minimise a window in this process (unbundled test host)"

    def menu_run(self, skips, reasons=None, names=None, drop=(), cancels=(), **options):
        """cancels are tests cancelled as they start, as RequiresDisplayFrames
        cancels one when the display fell asleep after the run was planned."""
        if names is None:
            names = (*self.NATIVE_TESTS, "ordinary()")
        if reasons is None:
            reasons = {}
        tests = [f"{native_target(name)}.Suite{i}/{name}" for i, name in enumerate(names)]
        ids = dict(zip(names, tests))
        records = finished(*(test for name, test in zip(names, tests) if name not in skips + drop + cancels))
        for name in skips:
            reason = reasons.get(name, "No macOS display is available")
            records.append(event("testSkipped", ids[name], f'Test {name} skipped: "{reason}"'))
        for name in cancels:
            reason = reasons.get(name, self.ASLEEP)
            records += cancelled(ids[name], name, reason)
        return self.run_gate(records + target_summaries(tests), inventory="\n".join(tests),
                             package=NATIVE_PACKAGE, **options)

    def native_test_runs(self, result):
        """The gate's test invocations, by the target their filter selects."""
        runs = {}
        for argv in result.invocations:
            if "--event-stream-output-path" not in argv:
                continue
            self.assertIn("--filter", argv)
            pattern = argv[argv.index("--filter") + 1]
            selected = [target for target in (NATIVE, INTELLIGENT_UI) if re.search(pattern, f"{target}.Suite/x()")]
            self.assertEqual(len(selected), 1, argv)
            self.assertNotIn(selected[0], runs)
            runs[selected[0]] = argv
        return runs

    def test_intelligent_ui_target_runs_serially(self):
        result = self.menu_run(())
        self.assertEqual(result.returncode, 0, result.stderr)
        runs = self.native_test_runs(result)
        self.assertEqual(sorted(runs), sorted([NATIVE, INTELLIGENT_UI]))
        self.assertIn("--no-parallel", runs[INTELLIGENT_UI])
        self.assertIn(f"{INTELLIGENT_UI}: ", result.stdout)
        # Discovery lists the whole package, unfiltered.
        listing = [argv for argv in result.invocations if "list" in argv]
        self.assertEqual(len(listing), 1)
        self.assertNotIn("--filter", listing[0])

    def test_native_target_stays_parallel(self):
        result = self.menu_run(())
        self.assertEqual(result.returncode, 0, result.stderr)
        runs = self.native_test_runs(result)
        self.assertEqual(sorted(runs), sorted([NATIVE, INTELLIGENT_UI]))
        self.assertNotIn("--no-parallel", runs[NATIVE])
        self.assertNotIn("--parallel", runs[NATIVE])

    def test_one_failing_invocation_fails_the_gate_whichever_it_is(self):
        # The other invocation passes, so a gate that kept only one
        # invocation's exit status, summary or completeness would pass.
        modes = {"exit": {"status": 1}, "summary": {"result": "failed"}, "timeout": {"sleep": 5}}
        for target in (NATIVE, INTELLIGENT_UI):
            for kind, mode in modes.items():
                with self.subTest(target=target, failure=kind):
                    result = self.menu_run((), per_target={target: mode},
                                           overrides={"RUN_TIMEOUT": 1.5})
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    if kind == "timeout":
                        self.assertIn(f"INCOMPLETE: timed out after 1.5s waiting for {NATIVE_PACKAGE} {target}",
                                      result.stderr)
                        self.assertNotIn("complete ", result.stdout.replace("INCOMPLETE", ""))
                    else:
                        line = result.stdout.strip().splitlines()[-1]
                        status = 1 if kind == "exit" else 0
                        verdict = "failed" if kind == "summary" else "passed"
                        self.assertIn(f"({target}: summary {verdict}, runner exit {status})", line)
                        other = INTELLIGENT_UI if target == NATIVE else NATIVE
                        self.assertNotIn(f"{other}: summary", line)

    def test_filtered_runs_set_aside_one_zero_test_summary_per_other_target(self):
        # The real shape: each filtered invocation's event stream has its own
        # target's summary plus a passing zero-test summary from the other.
        result = self.menu_run(())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("summary passed; runner exit 0\n", result.stdout)
        for target in (NATIVE, INTELLIGENT_UI):
            count = sum(native_target(name) == target for name in (*self.NATIVE_TESTS, "ordinary()"))
            for mode, message in [
                ({"empty": 0}, "expected 1 passing zero-test summaries from filtered-out test targets, found 0"),
                ({"empty": 2}, "expected 1 passing zero-test summaries from filtered-out test targets, found 2"),
                ({"empty_result": "failed"}, "found 0"),
                ({"count": count - 1}, f"summary reports {count - 1}/{count} declared tests"),
                ({"count": 0}, "found 2"),
            ]:
                with self.subTest(target=target, mode=mode):
                    result = self.menu_run((), per_target={target: mode})
                    self.assertEqual(result.returncode, 1, result.stdout)
                    self.assertIn(f"INCOMPLETE: {target}: ", result.stderr)
                    self.assertIn(message, result.stderr)

    def test_gate_deadline_names_the_running_invocation(self):
        result = self.menu_run((), per_target={INTELLIGENT_UI: {"sleep": 5}},
                               overrides={"GATE_TIMEOUT": 2.5})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertRegex(result.stderr, rf"INCOMPLETE: total gate deadline reached while running "
                                        rf"\S+ {INTELLIGENT_UI} for [0-9]+s \(2.5s shared by the whole package\)")

    def test_a_test_missing_from_either_invocation_fails(self):
        for name in ("ordinary()", self.REDUCED_MOTION_TESTS[0]):
            target = native_target(name)
            with self.subTest(target=target):
                result = self.menu_run((), drop=(name,))
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f"INCOMPLETE: {target}: finished", result.stderr)
                self.assertIn(name, result.stderr.split("missing=")[1].split("]")[0])

    def test_a_test_in_both_invocations_fails(self):
        names = (*self.NATIVE_TESTS, "ordinary()")
        for name, invocation in (("ordinary()", INTELLIGENT_UI), (self.REDUCED_MOTION_TESTS[0], NATIVE)):
            test = f"{native_target(name)}.Suite{names.index(name)}/{name}"
            with self.subTest(test=test, also_in=invocation):
                result = self.menu_run((), also={invocation: finished(test)})
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f"INCOMPLETE: {invocation}: finished", result.stderr)
                self.assertIn(f"unexpected=['{test}']", result.stderr)

    def test_every_native_test_target_needs_an_invocation(self):
        result = self.menu_run((), names=(*self.NATIVE_TESTS, "ordinary()"))
        self.assertEqual(result.returncode, 0, result.stderr)
        inventory = "\n".join([f"{native_target(name)}.Suite/{name}" for name in self.NATIVE_TESTS]
                              + ["OpenUINewTests.Suite/added()"])
        result = self.run_gate([], inventory=inventory, package=NATIVE_PACKAGE)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("no invocation runs the test targets ['OpenUINewTests']", result.stderr)
        self.assertEqual([argv for argv in result.invocations if "list" not in argv], [])

    def test_a_target_listed_twice_in_native_runs_fails(self):
        gate = runpy.run_path(str(SCRIPT))
        tests = [f"{NATIVE}.Suite/one()", f"{INTELLIGENT_UI}.Suite/two()"]
        self.assertEqual([run[2] for run in gate["test_runs"](NATIVE_PACKAGE, tests)], [tests[:1], tests[1:]])
        gate["test_runs"].__globals__["NATIVE_RUNS"] += ((INTELLIGENT_UI, ()),)
        with self.assertRaisesRegex(gate["IncompleteRun"], "exactly once"):
            gate["test_runs"](NATIVE_PACKAGE, tests)

    def test_callers_cannot_filter_or_change_parallelism(self):
        for options in (["--filter", "x"], ["--skip=x"], ["--no-parallel"], ["--parallel"], ["--num-workers", "1"]):
            with self.subTest(options=options):
                result = self.menu_run((), options=options)
                self.assertEqual(result.returncode, 2, result.stdout)
                self.assertIn("are not allowed", result.stderr)
                self.assertEqual(result.invocations, [])

    def test_headless_requires_all_named_skips(self):
        result = self.menu_run(self.DISPLAY_TESTS, screen="absent")
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in self.DISPLAY_TESTS:
            with self.subTest(missing=name):
                skips = tuple(test for test in self.DISPLAY_TESTS if test != name)
                self.assertNotEqual(self.menu_run(skips, screen="absent").returncode, 0)
        for skips in [(), self.DISPLAY_TESTS + ("ordinary()",),
                      self.DISPLAY_TESTS[:-1] + ("ordinary()",), self.DISPLAY_TESTS[:-1] + self.DISPLAY_TESTS[:1]]:
            with self.subTest(skips=skips):
                self.assertNotEqual(self.menu_run(skips, screen="absent").returncode, 0)

    def test_display_and_github_require_zero_skips(self):
        for options in [{"screen": "present"}, {"screen": "absent", "github": True},
                        {"screen": "asleep", "github": True},
                        {"screen": "present", "minimises": False, "github": True},
                        {"screen": "present", "locked": True}]:
            with self.subTest(options=options):
                self.assertEqual(self.menu_run((), **options).returncode, 0)
                for skips in [self.DISPLAY_TESTS[:1], self.DISPLAY_TESTS, self.MINIMISE_TESTS]:
                    self.assertNotEqual(self.menu_run(skips, **options).returncode, 0)

    def test_a_refused_minimise_requires_only_the_minimise_tests_to_skip_naming_it(self):
        refused = {name: self.REFUSED for name in self.MINIMISE_TESTS}
        for locked in [False, True]:
            with self.subTest(locked=locked):
                result = self.menu_run(self.MINIMISE_TESTS, minimises=False, locked=locked, reasons=refused)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("AppKit refuses a probe window", result.stdout)
                # Every other test still runs.
                for skips in [(), self.MINIMISE_TESTS + self.DISPLAY_TESTS[:1], self.MINIMISE_TESTS + ("ordinary()",),
                              self.MINIMISE_TESTS + self.DISPLAY_TESTS]:
                    with self.subTest(skips=skips):
                        result = self.menu_run(skips, minimises=False, locked=locked, reasons=refused)
                        self.assertNotEqual(result.returncode, 0)

    def test_a_refused_minimise_skip_belongs_to_the_intelligent_ui_invocation(self):
        refused = {name: self.REFUSED for name in self.MINIMISE_TESTS}
        result = self.menu_run(self.MINIMISE_TESTS, minimises=False, reasons=refused)
        self.assertEqual(result.returncode, 0, result.stderr)
        runs = self.native_test_runs(result)
        self.assertEqual(set(runs), {NATIVE, INTELLIGENT_UI})
        self.assertIn("--no-parallel", runs[INTELLIGENT_UI])
        # The skip's event comes only through that target's filtered stream, so
        # an Intelligent UI invocation that ran the test instead fails the gate.
        result = self.menu_run((), minimises=False, reasons=refused)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn(f"INCOMPLETE: {INTELLIGENT_UI}: ", result.stderr)
        self.assertIn(f"missing=['{self.MINIMISE_TESTS[0]}']", result.stderr)

    def test_refused_minimise_skips_must_name_the_measured_cause(self):
        for reason in ["No macOS display is available", self.ASLEEP, "session locked: AppKit will not minimise",
                       self.REFUSED + "."]:
            reasons = {name: reason for name in self.MINIMISE_TESTS}
            with self.subTest(reason=reason):
                result = self.menu_run(self.MINIMISE_TESTS, minimises=False, reasons=reasons)
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f'{self.MINIMISE_TESTS[0]} must skip with reason "{self.REFUSED}"', result.stderr)

    def test_a_minimising_host_runs_the_minimise_tests_even_when_locked(self):
        refused = {name: self.REFUSED for name in self.MINIMISE_TESTS}
        for options in [{"screen": "present"}, {"screen": "present", "locked": True},
                        {"screen": "asleep", "locked": True}]:
            with self.subTest(options=options):
                asleep = options["screen"] == "asleep"
                skips = self.MINIMISE_TESTS + (self.FRAME_TESTS if asleep else ())
                reasons = {**refused, **{name: self.ASLEEP for name in self.FRAME_TESTS}}
                result = self.menu_run(skips, reasons=reasons, **options)
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f"unexpected=['{self.MINIMISE_TESTS[0]}']", result.stderr)

    def test_asleep_and_refused_require_both_kinds_of_skip(self):
        reasons = {**{name: self.REFUSED for name in self.MINIMISE_TESTS},
                   **{name: self.ASLEEP for name in self.FRAME_TESTS}}
        result = self.menu_run(self.FRAME_TESTS + self.MINIMISE_TESTS, screen="asleep", minimises=False,
                               reasons=reasons)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("macOS display asleep; session unlocked; AppKit refuses a probe window", result.stdout)
        for skips in [self.FRAME_TESTS, self.MINIMISE_TESTS, self.FRAME_TESTS + self.MINIMISE_TESTS + self.MENU_TESTS[:1]]:
            with self.subTest(skips=skips):
                result = self.menu_run(skips, screen="asleep", minimises=False, reasons=reasons)
                self.assertNotEqual(result.returncode, 0)

    def test_headless_and_refused_require_both_kinds_of_skip(self):
        reasons = {name: self.REFUSED for name in self.MINIMISE_TESTS}
        both = self.DISPLAY_TESTS + self.MINIMISE_TESTS
        result = self.menu_run(both, screen="absent", minimises=False, reasons=reasons)
        self.assertEqual(result.returncode, 0, result.stderr)
        for skips in [self.DISPLAY_TESTS, self.MINIMISE_TESTS]:
            with self.subTest(skips=skips):
                result = self.menu_run(skips, screen="absent", minimises=False, reasons=reasons)
                self.assertNotEqual(result.returncode, 0)

    def test_the_minimise_tests_must_be_distinguishable_in_discovery(self):
        for names in [self.DISPLAY_TESTS, self.DISPLAY_TESTS + self.MINIMISE_TESTS * 2]:
            with self.subTest(count=len(names)):
                result = self.menu_run((), names=names)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("exactly once", result.stderr)

    def test_a_display_probe_without_every_fact_fails_the_gate(self):
        for probe in ["present", "present locked", "asleep unlocked", "present maybe minimises",
                      "present locked maybe", "locked present minimises", "present locked minimises extra"]:
            with self.subTest(probe=probe):
                result = self.menu_run((), probe=probe)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("determine whether a macOS display is available", result.stderr)

    def test_asleep_display_requires_frame_tests_to_skip_naming_the_sleep(self):
        asleep = {name: self.ASLEEP for name in self.FRAME_TESTS}
        result = self.menu_run(self.FRAME_TESTS, screen="asleep", reasons=asleep)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("macOS display asleep", result.stdout)
        for name in self.FRAME_TESTS:
            with self.subTest(missing=name):
                skips = tuple(test for test in self.FRAME_TESTS if test != name)
                self.assertNotEqual(self.menu_run(skips, screen="asleep", reasons=asleep).returncode, 0)
        # Menu tests need no frames, so they still run beside a sleeping display.
        for skips in [(), self.DISPLAY_TESTS, self.FRAME_TESTS + self.MENU_TESTS[:1],
                      self.FRAME_TESTS + ("ordinary()",)]:
            with self.subTest(skips=skips):
                self.assertNotEqual(self.menu_run(skips, screen="asleep", reasons=asleep).returncode, 0)

    # The frame tests sample the display as each starts, so one that falls
    # asleep during the run cancels the frame tests after it, naming the
    # sleep. The gate probes again after the run and accepts those.
    def test_a_display_that_sleeps_during_the_run_allows_named_frame_cancels(self):
        later = self.FRAME_TESTS[3:]
        self.assertIn("horizontalSwipeMovesTheCarouselToAnotherItem(attempt:)", later)
        for cancels in [later, self.FRAME_TESTS, ()]:
            with self.subTest(cancelled=len(cancels)):
                result = self.menu_run((), cancels=cancels, screen="present", probe_after="asleep unlocked minimises")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("macOS display present; session unlocked", result.stdout)
                self.assertIn("macOS display asleep after the run", result.stdout)

    def test_a_display_that_wakes_during_the_run_allows_frame_tests_to_run(self):
        asleep = {name: self.ASLEEP for name in self.FRAME_TESTS}
        for skips in [self.FRAME_TESTS, self.FRAME_TESTS[:5], ()]:
            with self.subTest(skipped=len(skips)):
                result = self.menu_run(skips, reasons=asleep, screen="asleep",
                                       probe_after="present unlocked minimises")
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_frame_cancels_during_the_run_must_name_the_sleep(self):
        for name in [self.FRAME_TESTS[0], "horizontalSwipeMovesTheCarouselToAnotherItem(attempt:)"]:
            with self.subTest(name=name):
                result = self.menu_run((), cancels=(name,), reasons={name: "No macOS display is available"},
                                       screen="present", probe_after="asleep unlocked minimises")
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f'{name} must skip with reason "{self.ASLEEP}"', result.stderr)

    def test_only_frame_tests_may_skip_when_the_display_sleeps_during_the_run(self):
        for name in [self.MENU_TESTS[0], self.REDUCED_MOTION_TESTS[0], "ordinary()"]:
            with self.subTest(name=name):
                result = self.menu_run((), cancels=self.FRAME_TESTS[3:] + (name,), screen="present",
                                       probe_after="asleep unlocked minimises")
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f"unexpected=['{name}']", result.stderr)

    def test_frame_cancels_with_the_display_awake_before_and_after_fail_naming_the_sleep(self):
        result = self.menu_run((), cancels=self.FRAME_TESTS[3:], screen="present")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("display slept during run", result.stderr)
        self.assertIn("display was present before and after the run", result.stderr)

    def test_a_display_that_disappears_during_the_run_is_incomplete(self):
        result = self.menu_run((), screen="present", probe_after="absent unlocked minimises")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("display was present before the run and absent after it", result.stderr)

    def test_the_display_is_probed_again_only_after_the_frame_tests_invocation(self):
        result = self.menu_run((), screen="present")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.count("after the run"), 1)
        for github in [True]:
            result = self.menu_run((), screen="present", github=github)
            self.assertNotIn("macOS display", result.stdout)

    def test_asleep_skips_must_name_the_sleeping_display(self):
        for name in self.FRAME_TESTS:
            reasons = {test: self.ASLEEP for test in self.FRAME_TESTS}
            reasons[name] = "No macOS display is available"
            with self.subTest(name=name):
                result = self.menu_run(self.FRAME_TESTS, screen="asleep", reasons=reasons)
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f'{name} must skip with reason "{self.ASLEEP}"', result.stderr)

    def test_all_display_tests_must_be_distinguishable_in_discovery(self):
        for names in [self.DISPLAY_TESTS[:-1], self.DISPLAY_TESTS + self.DISPLAY_TESTS[:1]]:
            with self.subTest(names=names):
                result = self.menu_run((), names=names)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("exactly once", result.stderr)

    def test_reduced_motion_variants_must_each_be_discovered_once(self):
        for name in self.REDUCED_MOTION_TESTS:
            others = tuple(test for test in self.NATIVE_TESTS if test != name)
            for names in [others, self.NATIVE_TESTS + (name,)]:
                with self.subTest(name=name, count=names.count(name)):
                    result = self.menu_run((), names=names)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(f"expected {name} exactly once", result.stderr)

    def test_reduced_motion_variants_run_without_frames_or_a_display(self):
        asleep = {name: self.ASLEEP for name in self.FRAME_TESTS}
        for screen, skips, reasons in [("present", (), {}), ("asleep", self.FRAME_TESTS, asleep),
                                       ("absent", self.DISPLAY_TESTS, {}),
                                       ("absent", (), {}), ("asleep", (), {})]:
            github = not skips and screen != "present"
            with self.subTest(screen=screen, github=github):
                self.assertEqual(self.menu_run(skips, screen=screen, reasons=reasons, github=github).returncode, 0)
                for name in self.REDUCED_MOTION_TESTS:
                    reason = {**reasons, name: self.ASLEEP if screen == "asleep" else "No macOS display is available"}
                    result = self.menu_run(skips + (name,), screen=screen, reasons=reason, github=github)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(f"unexpected=['{name}']", result.stderr)

    def test_headless_skips_must_name_the_missing_display(self):
        for name in self.DISPLAY_TESTS:
            reason = "No macOS display is available."
            with self.subTest(name=name):
                result = self.menu_run(self.DISPLAY_TESTS, screen="absent", reasons={name: reason})
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(name, result.stderr)

    def test_unsuccessful_display_probe_fails_the_gate(self):
        result = self.menu_run((), screen="")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("determine whether a macOS display is available", result.stderr)

    def test_runner_deadline_includes_descendants_holding_partial_output(self):
        runner = """
import os
import sys
import time

if os.fork() == 0:
    if sys.platform == 'darwin':
        os.setpgid(0, 0)  # Like the SwiftPM test helper.
    print('Test hung() started.', end='', flush=True)
    time.sleep(8)
    print('child was still running', flush=True)
"""
        result = subprocess.run([sys.executable, "-c", RUN_WITH_SHORT_DEADLINE, str(SCRIPT), runner],
                                capture_output=True, text=True, timeout=6)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("timed out after 2s waiting for test summaries", result.stderr)
        self.assertIn("last output: Test hung() started.", result.stderr)

    def test_runner_deadline_includes_blocked_stdout(self):
        runner = "import os; os.write(1, b'x' * 1048576)"
        with subprocess.Popen([sys.executable, "-c", RUN_WITH_SHORT_DEADLINE, str(SCRIPT), runner],
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as gate:
            try:
                # Leave stdout unread until the process has exited.
                self.assertEqual(gate.wait(timeout=4), 1)
            finally:
                if gate.poll() is None:
                    gate.kill()
                _, stderr = gate.communicate(timeout=2)
        self.assertIn("timed out after 2s waiting for test summaries", stderr)

    def test_zero_exit_without_final_summary_fails(self):
        result = self.run_gate(finished(ONE))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INCOMPLETE", result.stderr)
        self.assertIn(f"missing=['{TWO}']", result.stderr)
        self.assertIn("final test summary", result.stderr)

    def test_starting_every_test_without_finishing_still_fails(self):
        result = self.run_gate([event("testStarted", ONE), event("testStarted", TWO), summary(2)])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("finished 0/2", result.stderr)

    def test_duplicate_final_summaries_cannot_pass(self):
        result = self.run_gate(finished(ONE, TWO) + [summary(2), summary(2)])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected one final test summary, found 2", result.stderr)

    def test_summary_with_fewer_tests_than_discovery_fails(self):
        result = self.run_gate(finished(ONE) + [summary(1)])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("summary reports 1/2", result.stderr)

    def test_missing_test_cannot_hide_behind_matching_summary_or_console(self):
        result = self.run_gate(finished(ONE) + [summary(2)], output="✔ Test two(_:) passed after 0.1 seconds.\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("finished 1/2", result.stderr)

    def test_parameter_cases_do_not_inflate_declared_test_count(self):
        result = self.run_gate(finished(ONE, TWO) + [event("testCaseEnded", TWO), summary(2)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("complete 2/2", result.stdout)

    def test_top_level_test_ids_are_included(self):
        result = self.run_gate(finished("Example.one()") + [summary(1)],
                               inventory="Example.one()\n")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_newly_declared_test_must_also_run(self):
        result = self.run_gate(finished(ONE, TWO) + [summary(2)], inventory=TWO_TARGETS)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("three()", result.stderr)

    def test_each_test_target_runs_with_its_own_summary(self):
        result = self.run_gate(finished(ONE, TWO) + [summary(2)] + finished(THREE) + [summary(1)],
                               inventory=TWO_TARGETS)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("complete 3/3", result.stdout)

    def test_a_missing_target_summary_fails(self):
        result = self.run_gate(finished(ONE, TWO, THREE) + [summary(3)], inventory=TWO_TARGETS)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected 2 final test summaries (one per test target), found 1", result.stderr)

    def test_one_failing_target_fails_the_run(self):
        result = self.run_gate(finished(ONE, TWO) + [summary(2)] + finished(THREE) + [summary(1, "failed")],
                               inventory=TWO_TARGETS)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("summary failed", result.stdout)

    def test_target_summaries_must_match_their_targets_counts(self):
        result = self.run_gate(finished(ONE, TWO, THREE) + [summary(3), summary(0)], inventory=TWO_TARGETS)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("summaries report [0, 3] tests; the test targets declare [1, 2]", result.stderr)

    def test_zero_exit_with_failed_summary_still_fails(self):
        result = self.run_gate(finished(ONE, TWO) + [summary(2, "failed")])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("complete 2/2", result.stdout)

    def test_nonzero_exit_is_not_hidden_by_passing_summary(self):
        result = self.run_gate(finished(ONE, TWO) + [summary(2)], status=7, output="REVIEW_ASCII_CONTROL\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REVIEW_ASCII_CONTROL", result.stdout)

    def test_empty_or_malformed_discovery_cannot_pass(self):
        for inventory in ["", "build failed\n"]:
            with self.subTest(inventory=inventory):
                self.assertNotEqual(self.run_gate([], inventory=inventory).returncode, 0)


if __name__ == "__main__":
    unittest.main()
