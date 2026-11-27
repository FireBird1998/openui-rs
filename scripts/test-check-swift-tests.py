#!/usr/bin/env python3
"""Exercise the Swift completeness gate, including successful-but-truncated runners."""
import json
import os
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


def summary(count, result="passed"):
    return event("runEnded", text=f"Test run with {count} tests in 1 suite {result} after 0.1 seconds.")


def event_log(events):
    return "".join(json.dumps(record) + "\n" for record in events)


class SwiftTestGate(unittest.TestCase):
    def test_total_deadline_bounds_cleanup_after_a_blocked_audit(self):
        # No process starts between the gate's start and the blocked audit:
        # discovery and the summary run are replaced in-process, so the 0.55 s
        # deadline cannot fire before the audit blocks, however slow the host.
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
globals_["GATE_TIMEOUT"] = 0.55
tempfile = globals_["tempfile"]

def run_swift(command, waiting_for, timeout, **options):
    if "list" in command:
        return 0, {ONE!r} + "\\n"
    os.mkfifo(command[-1])  # The audit's open() then blocks with no writer.
    return 0, ""

class SlowCleanup(tempfile.TemporaryDirectory):
    def __exit__(self, *args):
        Path(root, "cleanup-started").touch()
        time.sleep(2)
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
            self.assertTrue(any(path.is_fifo() for path in root.glob("swift-test-gate-*/events.jsonl")))
            self.assertTrue((root / "cleanup-started").is_file(), gate.stderr)
            self.assertIn("total gate deadline reached during cleanup", gate.stderr)
            # The hard exit comes 0.55 s after the gate starts; an uncut
            # cleanup would end at least 2.5 s after it.
            self.assertLess(ended - float((root / "gate-started").read_text()), 1.5)

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
                 screen="present", github=False, output="", stderr="", discovery_stderr="",
                 read_size=None, hang=False, log=None):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            swift = directory / "swift"
            if log is None:
                log = event_log(events)
            swift.write_text(f"#!{sys.executable}\nfrom pathlib import Path\nimport sys\nimport time\n"
                             f"if '-e' in sys.argv: print({screen!r}); sys.exit(0)\n"
                             "if 'list' in sys.argv:\n"
                             f"    print({inventory!r}, end='')\n"
                             f"    print({discovery_stderr!r}, end='', file=sys.stderr)\n"
                             "    sys.exit(0)\n"
                             f"print({output!r}, end='', flush=True)\n"
                             f"print({stderr!r}, end='', file=sys.stderr, flush=True)\n"
                             f"Path(sys.argv[sys.argv.index('--event-stream-output-path') + 1]).write_text({log!r})\n"
                             f"if {hang!r}: time.sleep(8)\n"
                             f"sys.exit({status})\n")
            swift.chmod(0o755)
            command = [sys.executable, str(SCRIPT), package]
            if read_size is not None:
                command = [sys.executable, "-c", RUN_GATE_WITH_CONTROLLED_READS, str(SCRIPT), package, str(read_size)]
            return subprocess.run(command,
                                  env={**os.environ, "GITHUB_ACTIONS": str(github).lower(),
                                       "PATH": str(directory) + os.pathsep + os.environ["PATH"]},
                                  capture_output=True, text=True, timeout=6)

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
