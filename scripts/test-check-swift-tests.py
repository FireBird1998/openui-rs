#!/usr/bin/env python3
"""Exercise the Swift completeness gate, including successful-but-truncated runners."""
import json
import os
from pathlib import Path
import runpy
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("check-swift-tests.py").resolve()
FIXTURES = SCRIPT.parent / "fixtures"
ONE = "Example.Parent/Child/one()"
TWO = "Example.Parent/Child/two(_:)"
THREE = "Other.Suite/three()"
TWO_TARGETS = f"{ONE}\n{TWO}\n{THREE}\n"
RUN_WITH_SHORT_DEADLINE = """
import runpy
import shutil
import signal
import sys

script, runner = sys.argv[1:]
gate = runpy.run_path(script)
try:
    gate['run_swift']([sys.executable, '-c', runner], 'test summaries', 2, stream=True)
except gate['IncompleteRun'] as error:
    sys.exit(str(error))
"""


def event(kind, test=None, text=None):
    payload = {"kind": kind, "messages": [{"text": text}] if text else []}
    if test is not None:
        payload["testID"] = f"{test}/Fixture.swift:1:1"
    return {"kind": "event", "payload": payload}


def finished(*tests):
    return [event("testEnded", test) for test in tests]


def summary(count, result="passed"):
    return event("runEnded", text=f"Test run with {count} tests in 1 suite {result} after 0.1 seconds.")


def event_log(events):
    return "".join(json.dumps(record) + "\n" for record in events)


class SwiftTestGate(unittest.TestCase):
    def run_viewer_gate(self, incomplete, *, interrupt=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            (root / "tools/viewer/Requests").mkdir(parents=True)
            for name in ["check-viewer.sh", "check-swift-tests.py"]:
                (root / "scripts" / name).write_text(SCRIPT.with_name(name).read_text())
            (root / "tools/viewer/Requests/sync-recorded-streams.py").write_text("")
            (root / "scripts/test-check-swift-tests.py").write_text("")
            binding = root / "scripts/viewer.sh"
            binding.write_text("#!/bin/sh\nexit 0\n")
            binding.chmod(0o755)
            swift = root / "swift"
            packages = {}
            for package in ("swift/OpenUINative", "tools/viewer", "tools/chat"):
                names = self.DISPLAY_TESTS if package == "swift/OpenUINative" else ("one()", "two()")
                tests = [f"Example.Suite/{name}" for name in names]
                records = (finished(*tests) + [summary(len(tests))] if package != incomplete
                           else [event("testStarted", tests[0])])
                packages[package] = ("\n".join(tests), event_log(records))
            swift.write_text(f"""#!{sys.executable}
from pathlib import Path
import os, signal, sys, time
if 'build' in sys.argv: sys.exit(0)
if '-e' in sys.argv: print('present'); sys.exit(0)
packages = {packages!r}
inventory, log = packages[sys.argv[sys.argv.index('--package-path') + 1]]
if 'list' in sys.argv:
    print(inventory)
elif {interrupt!r}:
    scratch = Path(os.environ["OPENUI_SWIFT_TEST_CAPTURE_REQUESTS"])
    pid = os.getpid()
    response = scratch / f"stack-{{pid}}.txt"
    signal.signal(signal.SIGUSR2, lambda *_: response.write_text("Test viewerInterrupt()\\n"))
    (scratch / f"ready-{{pid}}").touch()
    Path("interrupt-ready").touch()
    time.sleep(20)
else:
    Path(sys.argv[sys.argv.index('--event-stream-output-path') + 1]).write_text(log)
""")
            swift.chmod(0o755)
            command = ["bash", str(root / "scripts/check-viewer.sh")]
            env = {**os.environ, "PATH": str(root) + os.pathsep + os.environ["PATH"],
                   "OPENUI_SWIFT_TEST_STACKS": str(root / "stacks")}
            if not interrupt:
                return subprocess.run(command, env=env, capture_output=True, text=True)
            with subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as viewer:
                try:
                    deadline = time.monotonic() + 5
                    while not (root / "interrupt-ready").exists() and time.monotonic() < deadline:
                        time.sleep(0.02)
                    self.assertTrue((root / "interrupt-ready").exists())
                    viewer.send_signal(signal.SIGINT)
                    stdout, stderr = viewer.communicate(timeout=10)
                finally:
                    if viewer.poll() is None:
                        viewer.terminate()
                self.assertEqual([path.read_text() for path in (root / "stacks").iterdir()],
                                 ["Test viewerInterrupt()\n"])
                return subprocess.CompletedProcess(command, viewer.returncode, stdout, stderr)

    def test_viewer_interrupt_reaches_the_active_gate(self):
        result = self.run_viewer_gate(None, interrupt=True)
        self.assertEqual(result.returncode, 130, result.stderr)
        self.assertIn("interrupted by SIGTERM", result.stderr)
        self.assertIn("Test viewerInterrupt()", result.stderr)

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
        native_count = len(self.DISPLAY_TESTS)
        self.assertIn(f"complete {native_count}/{native_count}", result.stdout)
        self.assertEqual(result.stdout.count("complete 2/2"), 2)

    def run_gate(self, events, status=0, inventory=f"{ONE}\n{TWO}\n", package="example",
                 screen="present", github=False, output=""):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            swift = directory / "swift"
            swift.write_text(f"#!{sys.executable}\nfrom pathlib import Path\nimport sys\n"
                             f"if '-e' in sys.argv: print({screen!r}); sys.exit(0)\n"
                             f"if 'list' in sys.argv: print({inventory!r}, end=''); sys.exit(0)\n"
                             f"print({output!r}, end='')\n"
                             f"Path(sys.argv[sys.argv.index('--event-stream-output-path') + 1]).write_text({event_log(events)!r})\n"
                             f"sys.exit({status})\n")
            swift.chmod(0o755)
            return subprocess.run([sys.executable, str(SCRIPT), package],
                                  env={**os.environ, "GITHUB_ACTIONS": str(github).lower(),
                                       "PATH": str(directory) + os.pathsep + os.environ["PATH"]},
                                  capture_output=True, text=True)

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

    def menu_run(self, skips, reasons=None, names=None, **options):
        if names is None:
            names = (*self.DISPLAY_TESTS, "ordinary()")
        if reasons is None:
            reasons = {}
        tests = [f"OpenUINativeTests.Suite{i}/{name}" for i, name in enumerate(names)]
        ids = dict(zip(names, tests))
        records = finished(*(test for name, test in zip(names, tests) if name not in skips))
        for name in skips:
            reason = reasons.get(name, "No macOS display is available")
            records.append(event("testSkipped", ids[name], f'Test {name} skipped: "{reason}"'))
        return self.run_gate(records + [summary(len(names))], inventory="\n".join(tests),
                             package=SCRIPT.resolve().parents[1] / "swift/OpenUINative",
                             **options)

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
        for options in [{"screen": "present"}, {"screen": "absent", "github": True}]:
            with self.subTest(options=options):
                self.assertEqual(self.menu_run((), **options).returncode, 0)
                for skips in [self.DISPLAY_TESTS[:1], self.DISPLAY_TESTS]:
                    self.assertNotEqual(self.menu_run(skips, **options).returncode, 0)

    def test_all_display_tests_must_be_distinguishable_in_discovery(self):
        for names in [self.DISPLAY_TESTS[:-1], self.DISPLAY_TESTS + self.DISPLAY_TESTS[:1]]:
            with self.subTest(names=names):
                result = self.menu_run((), names=names)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("exactly once", result.stderr)

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

    def test_silence_arms_without_events_and_survives_stdout_eof(self):
        gate = runpy.run_path(str(SCRIPT))
        for runner in ("import time; time.sleep(20)", "import os,time; os.close(1); os.close(2); time.sleep(20)"):
            with self.subTest(runner=runner), tempfile.TemporaryDirectory() as scratch:
                with self.assertRaisesRegex(gate["IncompleteRun"], "after 0.2s waiting for output"):
                    gate["run_swift"]([sys.executable, "-c", runner], "test summaries", 4,
                                      event_path=Path(scratch) / "events.jsonl", silence_timeout=0.2)

    def test_silent_runner_is_named_and_sampled_before_it_is_killed(self):
        gate = runpy.run_path(str(SCRIPT))
        globals_ = gate["run_swift"].__globals__
        with tempfile.TemporaryDirectory() as scratch:
            events = Path(scratch) / "events.jsonl"
            stacks = Path(scratch) / "stacks"
            runner = ("from pathlib import Path\nimport time\n"
                      f"Path({str(events)!r}).write_text({event_log([event('testStarted', TWO)])!r})\n"
                      "time.sleep(20)\n")
            processes = []
            popen = subprocess.Popen

            def launch(args, **options):
                process = popen(args, **options)
                processes.append(process)
                return process

            def capture(process, directory, _requests):
                self.assertIsNone(process.poll())
                directory.mkdir()
                (directory / f"gate-{process.pid}-1.sample.txt").write_text("fixture stack\n")

            with patch("subprocess.Popen", side_effect=launch), \
                    patch.dict(globals_, capture_before_kill=capture), self.assertRaisesRegex(
                gate["IncompleteRun"], r"after 0.3s waiting for output from test summaries; unfinished tests: "
                r"Example.Parent/Child/two\(_:\)"
            ):
                gate["run_swift"]([sys.executable, "-c", runner], "test summaries", 10,
                                  event_path=events, stack_directory=stacks, silence_timeout=0.3)
            self.assertIsNotNone(processes[0].poll())
            self.assertEqual([path.read_text() for path in stacks.iterdir()], ["fixture stack\n"])

    def test_flooding_event_writer_is_killed_before_the_diagnostic_scan(self):
        gate = runpy.run_path(str(SCRIPT))
        globals_ = gate["run_swift"].__globals__
        with tempfile.TemporaryDirectory() as scratch:
            events = Path(scratch) / "events.jsonl"
            pid_file = Path(scratch) / "pid"
            runner = f"""
from pathlib import Path
import os, time
Path({str(pid_file)!r}).write_text(str(os.getpid()))
with open({str(events)!r}, "wb", buffering=0) as stream:
    while True:
        stream.write(b"{{}}\\n" * 8192)
        time.sleep(0.001)
"""

            def active(path):
                self.assertEqual(path, events)
                pid = int(pid_file.read_text())
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)
                self.assertGreater(events.stat().st_size, gate["EVENT_SNAPSHOT_BYTES"])
                return gate["active_tests"](path)

            with patch.dict(globals_, active_tests=active), self.assertRaisesRegex(
                    gate["IncompleteRun"], "after 0.3s waiting for test summaries"):
                gate["run_swift"]([sys.executable, "-c", runner], "test summaries", 0.3,
                                  event_path=events)

    def test_sampler_deadline_reaps_sampler_and_ignores_diagnostics(self):
        gate = runpy.run_path(str(SCRIPT))
        globals_ = gate["capture_stacks"].__globals__
        popen = subprocess.Popen
        samples = []

        def launch(args, **options):
            self.assertEqual(args[0], "/usr/bin/sample")
            process = popen([sys.executable, "-c",
                             "import sys,time; print('REVIEW_FAKE_SECRET_254', file=sys.stderr); time.sleep(20)"],
                            **options)
            samples.append(process)
            return process

        with tempfile.TemporaryDirectory() as scratch, patch("subprocess.Popen", side_effect=launch), \
                patch.dict(globals_, SAMPLE_TIMEOUT=0.2, swift_children=lambda _pid: {}):
            stacks = Path(scratch) / "stacks"
            gate["capture_stacks"](os.getpid(), stacks, Path(scratch))
            self.assertEqual(len(samples), 1)
            self.assertLess(samples[0].returncode, 0)
            self.assertEqual(list(stacks.iterdir()), [])

    def test_sampler_artifact_keeps_only_the_call_graph(self):
        gate = runpy.run_path(str(SCRIPT))
        popen = subprocess.Popen

        def launch(args, **options):
            self.assertEqual(args[0], "/usr/bin/sample")
            report = ("REVIEW_FAKE_SECRET_254 in metadata\nCall graph:\n    sample stack frame\n"
                      "Total number in stack\nREVIEW_FAKE_SECRET_254 in footer\n")
            return popen([sys.executable, "-c", f"from pathlib import Path; Path({args[4]!r}).write_text({report!r})"],
                         **options)

        with tempfile.TemporaryDirectory() as scratch, patch("subprocess.Popen", side_effect=launch):
            stacks = Path(scratch) / "stacks"
            gate["capture_stacks"](os.getpid(), stacks, Path(scratch))
            self.assertEqual([path.read_text() for path in stacks.iterdir()],
                             ["Call graph:\n    sample stack frame\n"])

    def test_hung_capture_worker_cannot_delay_kill(self):
        gate = runpy.run_path(str(SCRIPT))
        globals_ = gate["run_swift"].__globals__
        popen = subprocess.Popen
        processes = []

        def launch(args, **options):
            if "--capture-swift-stacks" in args:
                args = [sys.executable, "-c", "import time; time.sleep(20)"]
            process = popen(args, **options)
            processes.append(process)
            return process

        with tempfile.TemporaryDirectory() as scratch, patch("subprocess.Popen", side_effect=launch), \
                patch.dict(globals_, CAPTURE_TIMEOUT=0.2):
            began = time.monotonic()
            with self.assertRaises(gate["IncompleteRun"]):
                gate["run_swift"]([sys.executable, "-c", "import time; time.sleep(20)"], "test summaries", 0.2,
                                  event_path=Path(scratch) / "events.jsonl", stack_directory=Path(scratch) / "stacks")
            self.assertLess(time.monotonic() - began, 2)
            self.assertEqual(len(processes), 2)
            self.assertLess(processes[0].returncode, 0)
            self.assertLess(processes[1].wait(timeout=2), 0)

    def test_sigint_captures_before_kill_without_console_text_in_artifact(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            runner = r"""
import os, signal, time
from pathlib import Path
scratch = Path(os.environ["OPENUI_SWIFT_TEST_CAPTURE_REQUESTS"])
pid = os.getpid()
signal.signal(signal.SIGUSR2, lambda *_: (scratch / f"stack-{pid}.txt").write_text("Test interruptedFixture()\nstack\n"))
print("REVIEW_FAKE_SECRET_254", flush=True)
(scratch / f"ready-{pid}").touch()
time.sleep(20)
"""
            launcher = """
import runpy, sys
from pathlib import Path
script, directory, runner = sys.argv[1:]
gate = runpy.run_path(script)
try:
    gate["run_swift"]([sys.executable, "-c", runner], "test summaries", 15, stream=True,
                      event_path=Path(directory) / "events.jsonl", stack_directory=Path(directory) / "stacks")
except gate["IncompleteRun"] as error:
    sys.exit(str(error))
"""
            with subprocess.Popen([sys.executable, "-c", launcher, str(SCRIPT), scratch, runner],
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as gate:
                try:
                    self.assertEqual(gate.stdout.readline().strip(), "REVIEW_FAKE_SECRET_254")
                    deadline = time.monotonic() + 3
                    while not list(root.glob("ready-*")) and time.monotonic() < deadline:
                        time.sleep(0.02)
                    gate.send_signal(signal.SIGINT)
                    _, stderr = gate.communicate(timeout=10)
                finally:
                    if gate.poll() is None:
                        gate.terminate()
            self.assertIn("interrupted by SIGINT", stderr)
            self.assertIn("Test interruptedFixture()", stderr)
            self.assertEqual([path.read_text() for path in (root / "stacks").iterdir()],
                             ["Test interruptedFixture()\nstack\n"])

    def test_completed_work_renews_silence_and_run_deadlines(self):
        gate = runpy.run_path(str(SCRIPT))
        with tempfile.TemporaryDirectory() as scratch:
            runner = """
import os, time
from pathlib import Path
progress = Path(os.environ["OPENUI_SWIFT_TEST_CAPTURE_REQUESTS"]) / "progress"
for _ in range(8):
    progress.touch()
    time.sleep(0.09)
"""
            status, _ = gate["run_swift"]([sys.executable, "-c", runner], "test summaries", 0.3,
                                         event_path=Path(scratch) / "events.jsonl", silence_timeout=0.23)
            self.assertEqual(status, 0)

    def test_chart_and_carousel_stress_allow_the_full_native_watchdog_window(self):
        gate = runpy.run_path(str(SCRIPT))
        env = {"OPENUI_CHART_ABSOLUTE_MAX": "", "OPENUI_REPEAT_CAROUSEL_OVERFLOW": "",
               "OPENUI_REPEAT_CAROUSEL_SWIPE": ""}
        package = gate["NATIVE_PACKAGE"]
        with patch.dict(os.environ, env):
            self.assertGreater(gate["silence_budget"](package), 175 + 10)
        for name in env:
            with self.subTest(name=name), patch.dict(os.environ, {**env, name: "1" if "CHART" in name else "400"}):
                self.assertGreater(gate["silence_budget"](package), 240 + 10)

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



class TextReproduction(unittest.TestCase):
    gate = runpy.run_path(str(SCRIPT))
    all_tests = gate["TEXT_REPRO_TESTS"] + gate["PARALLEL_REPRO_TESTS"]

    def records(self, tests, repetitions, *, case_events=True, duplicate=False, empty_target=False):
        ids = self.gate["repro_ids"](tests)
        records = [summary(0)] if empty_target else []
        records.extend(event("testStarted", test) for test in ids)
        proof = []
        for test, name in ids.items():
            for iteration in range(1, repetitions + 1):
                if case_events:
                    for kind, instant in (("testCaseStarted", 1), ("testCaseEnded", 2)):
                        record = event(kind, test)
                        record["payload"].update(iteration=1 if duplicate else iteration,
                                                 instant={"absolute": instant})
                        records.append(record)
                proof.append(json.dumps({"test": name, "iteration": iteration, "elapsed": 0.02}))
        records.extend(finished(*ids))
        records.append(summary(len(ids)))
        return event_log(records).splitlines(), proof

    def test_runner_2084_log_counts_ten_completed_repetitions(self):
        output = (FIXTURES / "swift-testing-2084-repetitions.txt").read_text()
        names = {name + "()" for name in self.all_tests}
        self.assertEqual(self.gate["legacy_repetition_results"](output, names, 10), dict.fromkeys(names, 10))
        records, proof = self.records(self.all_tests, 10, case_events=False)
        counts, _ = self.gate["repetition_results"](records, proof, output, self.all_tests, 10)
        self.assertEqual(counts, dict.fromkeys(names, 10))
        missing = output.replace(" started (repetition 10).", " started (repetition 9).", 1)
        with self.assertRaises(self.gate["IncompleteRun"]):
            self.gate["repetition_results"](records, proof, missing, self.all_tests, 10)

    def test_duplicate_case_pairs_are_rejected_even_with_passing_summary(self):
        records, proof = self.records(self.all_tests, 2, duplicate=True)
        with self.assertRaisesRegex(self.gate["IncompleteRun"], "duplicate or unmatched repetition"):
            self.gate["repetition_results"](records, proof, "", self.all_tests, 2)

    def test_fake_name_tests_without_body_completions_are_rejected(self):
        records, _ = self.records(self.all_tests, 2)
        with self.assertRaisesRegex(self.gate["IncompleteRun"], "missing test-body repetition completions"):
            self.gate["repetition_results"](records, [], "", self.all_tests, 2)
        result = subprocess.run([sys.executable, str(SCRIPT), "--reproduce-text", "2", "--package-path", "other"],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("fixed test selection", result.stderr)

    def test_a_single_repetition_needs_only_function_events_and_body_completion(self):
        records, proof = self.records(self.all_tests, 1, case_events=False, empty_target=True)
        counts, timings = self.gate["repetition_results"](records, proof, "", self.all_tests, 1)
        self.assertEqual(counts, dict.fromkeys((name + "()" for name in self.all_tests), 1))
        self.assertEqual(len(timings), len(self.all_tests))


@unittest.skipUnless(sys.platform == "darwin" and shutil.which("swift"), "requires macOS Swift Testing")
class NativeWatchdogFixtures(unittest.TestCase):
    def test_wait_names_progress_and_exit_stacks(self):
        support = SCRIPT.parents[1] / "swift/OpenUINative/Tests"
        with tempfile.TemporaryDirectory(prefix="native-watchdog-") as scratch:
            root = Path(scratch)
            for name in ("SwiftTestSupport", "TestThreadBacktraces"):
                shutil.copytree(support / name, root / name)
            fixture = root / "Fixture"
            fixture.mkdir()
            shutil.copy(FIXTURES / "swift-test-watchdog.swift", fixture / "Watchdog.swift")
            (root / "Package.swift").write_text("""// swift-tools-version: 6.0
import PackageDescription
let package = Package(name: "WatchdogFixtures", platforms: [.macOS("27.0")], targets: [
    .target(name: "TestThreadBacktraces", path: "TestThreadBacktraces"),
    .target(name: "SwiftTestSupport", dependencies: ["TestThreadBacktraces"], path: "SwiftTestSupport"),
    .testTarget(name: "Fixture", dependencies: ["SwiftTestSupport"], path: "Fixture")
])
""")
            command = ["swift", "test", "--package-path", scratch, "--disable-sandbox", "-Xswiftc", "-warnings-as-errors"]
            env = {**os.environ, "TMPDIR": scratch}
            for name, operation in (("continuousProgress", None),
                                    ("blockedDisplay", "AppKit to enumerate macOS displays"),
                                    ("namedOperationStalls", "Core to load the schema fixture"),
                                    ("stalledExitChild", "waiting for the helper to return")):
                with self.subTest(name=name):
                    stacks = root / name
                    result = subprocess.run([*command, "--filter", name],
                                            env={**env, "OPENUI_SWIFT_TEST_STACKS": str(stacks)},
                                            capture_output=True, text=True, timeout=60)
                    console = result.stdout + result.stderr
                    if operation is None:
                        self.assertEqual(result.returncode, 0, console)
                        self.assertIn("Test continuousProgress() passed", console)
                    else:
                        self.assertNotEqual(result.returncode, 0, console)
                        self.assertIn(operation, console)
                        self.assertIn("In-process backtraces", console)
                        captures = list(stacks.rglob("*.backtrace.txt"))
                        self.assertEqual(len(captures), 1, console)
                        self.assertIn(f"Test {name}()", captures[0].read_text())
                        if name == "stalledExitChild":
                            self.assertIn("Swift test watchdog: stack in", console)


if __name__ == "__main__":
    unittest.main()
