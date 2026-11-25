#!/usr/bin/env python3
"""Exercise the Swift completeness gate, including successful-but-truncated runners."""
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("check-swift-tests.py")
ONE = "Example.Parent/Child/one()"
TWO = "Example.Parent/Child/two(_:)"
THREE = "Other.Suite/three()"
TWO_TARGETS = f"{ONE}\n{TWO}\n{THREE}\n"
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
    def run_viewer_gate(self, incomplete):
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
            swift.write_text(f"#!{sys.executable}\nfrom pathlib import Path\nimport sys\n"
                             "if 'build' in sys.argv: sys.exit(0)\n"
                             "if '-e' in sys.argv: print('present'); sys.exit(0)\n"
                             f"packages = {packages!r}\n"
                             "inventory, log = packages[sys.argv[sys.argv.index('--package-path') + 1]]\n"
                             "if 'list' in sys.argv:\n"
                             "    print(inventory)\n"
                             "else:\n"
                             "    Path(sys.argv[sys.argv.index('--event-stream-output-path') + 1]).write_text(log)\n")
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

    def test_silent_runner_is_named_and_sampled_before_it_is_killed(self):
        gate = runpy.run_path(str(SCRIPT))
        with tempfile.TemporaryDirectory() as scratch:
            events = Path(scratch) / "events.jsonl"
            stacks = Path(scratch) / "stacks"
            runner = ("from pathlib import Path\nimport time\n"
                      f"Path({str(events)!r}).write_text({event_log([event('testStarted', TWO)])!r})\n"
                      "time.sleep(20)\n")
            processes = []
            popen = subprocess.Popen

            def launch(args, **options):
                if args[0] == "/usr/bin/sample":
                    self.assertIsNone(processes[0].poll())
                    self.assertEqual(args[1:4], [str(processes[0].pid), "5", "-file"])
                    sample = f"from pathlib import Path; Path({args[4]!r}).write_text('fixture stack\\n')"
                    return popen([sys.executable, "-c", sample], **options)
                process = popen(args, **options)
                processes.append(process)
                return process

            with patch("subprocess.Popen", side_effect=launch), self.assertRaisesRegex(
                gate["IncompleteRun"], r"after 0.2s waiting for output from test summaries; unfinished tests: "
                r"Example.Parent/Child/two\(_:\)"
            ):
                gate["run_swift"]([sys.executable, "-c", runner], "test summaries", 10,
                                  event_path=events, stack_directory=stacks, silence_timeout=0.2)
            self.assertIsNotNone(processes[0].poll())
            self.assertEqual([path.read_text() for path in stacks.iterdir()], ["fixture stack\n"])

    def test_sampler_deadline_does_not_leave_a_sampler_running(self):
        gate = runpy.run_path(str(SCRIPT))
        gate["capture_stacks"].__globals__["SAMPLE_TIMEOUT"] = 0.2
        popen = subprocess.Popen
        samples = []

        def launch(args, **options):
            self.assertEqual(args[0], "/usr/bin/sample")
            process = popen([sys.executable, "-c", "import time; time.sleep(20)"], **options)
            samples.append(process)
            return process

        with tempfile.TemporaryDirectory() as scratch, patch("subprocess.Popen", side_effect=launch):
            gate["capture_stacks"](type("TestProcess", (), {"pid": os.getpid()})(), Path(scratch), "fixture timeout")
            self.assertEqual(len(samples), 1)
            self.assertLess(samples[0].returncode, 0)
            self.assertIn("sample timed out after 0.2s", next(Path(scratch).iterdir()).read_text())

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
