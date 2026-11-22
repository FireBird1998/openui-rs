#!/usr/bin/env python3
"""Exercise the Swift completeness gate, including successful-but-truncated runners."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("check-swift-tests.py")


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
            swift.write_text(f"#!{sys.executable}\nimport sys\n"
                             "if 'build' in sys.argv: sys.exit(0)\n"
                             "if 'list' in sys.argv:\n"
                             "    print('Example.Suite/one()\\nExample.Suite/two()')\n"
                             "else:\n"
                             "    print('◇ Test one() started.')\n"
                             f"    if {incomplete!r} not in sys.argv:\n"
                             "        print('◇ Test two() started.\\n✔ Test run with 2 tests passed after 0.1 seconds.')\n")
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
        self.assertEqual(result.stdout.count("complete 2/2"), 3)

    def run_gate(self, output, status=0, inventory="Example.Parent/Child/one()\nExample.Parent/Child/two(_:)\n"):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            swift = directory / "swift"
            swift.write_text(f"#!{sys.executable}\nimport sys\n"
                             f"print({inventory!r} if 'list' in sys.argv else {output!r}, end='')\n"
                             f"sys.exit(0 if 'list' in sys.argv else {status})\n")
            swift.chmod(0o755)
            return subprocess.run([sys.executable, str(SCRIPT), "example"],
                                  env={**os.environ, "PATH": str(directory) + os.pathsep + os.environ["PATH"]},
                                  capture_output=True, text=True)

    def test_zero_exit_without_final_summary_fails(self):
        result = self.run_gate("◇ Test one() started.\n✔ Test one() passed after 0.1 seconds.\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("INCOMPLETE", result.stderr)
        self.assertIn("missing=['two(_:)']", result.stderr)
        self.assertIn("final test summary", result.stderr)

    def test_starting_every_test_without_finishing_still_fails(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected one final test summary, found 0", result.stderr)

    def test_duplicate_final_summaries_cannot_pass(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n"
                               "✔ Test run with 2 tests passed after 0.1 seconds.\n"
                               "✔ Test run with 2 tests passed after 0.1 seconds.\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected one final test summary, found 2", result.stderr)

    def test_summary_with_fewer_tests_than_discovery_fails(self):
        result = self.run_gate("◇ Test one() started.\n✔ Test run with 1 test passed after 0.1 seconds.\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("summary reports 1/2", result.stderr)

    def test_missing_start_cannot_hide_behind_matching_summary(self):
        result = self.run_gate("◇ Test one() started.\n✔ Test run with 2 tests passed after 0.1 seconds.\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("started 1/2", result.stderr)

    def test_parameter_cases_do_not_inflate_declared_test_count(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n"
                               "◇ Test case passing 1 argument x → 2 to two(_:) started.\n"
                               "✔ Test run with 2 tests in 2 suites passed after 0.1 seconds.\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("complete 2/2", result.stdout)

    def test_top_level_test_ids_are_included(self):
        result = self.run_gate("◇ Test one() started.\n✔ Test run with 1 test passed after 0.1 seconds.\n",
                               inventory="Example.one()\n")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_newly_declared_test_must_also_run(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n"
                               "✔ Test run with 2 tests passed after 0.1 seconds.\n",
                               inventory="Example/one()\nExample/two(_:)\nExample/three()\n".replace("Example/", "Example.Suite/"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("three()", result.stderr)

    TWO_TARGETS = "Example.Suite/one()\nExample.Suite/two(_:)\nOther.Suite/three()\n"

    def test_each_test_target_runs_with_its_own_summary(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n"
                               "✔ Test run with 2 tests in 1 suite passed after 0.1 seconds.\n"
                               "◇ Test three() started.\n"
                               "✔ Test run with 1 test in 1 suite passed after 0.1 seconds.\n",
                               inventory=self.TWO_TARGETS)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("complete 3/3", result.stdout)

    def test_a_missing_target_summary_fails(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n◇ Test three() started.\n"
                               "✔ Test run with 3 tests passed after 0.1 seconds.\n",
                               inventory=self.TWO_TARGETS)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected 2 final test summaries (one per test target), found 1", result.stderr)

    def test_one_failing_target_fails_the_run(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n"
                               "✔ Test run with 2 tests passed after 0.1 seconds.\n"
                               "◇ Test three() started.\n"
                               "✘ Test run with 1 test failed after 0.1 seconds with 1 issue.\n",
                               inventory=self.TWO_TARGETS)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("summary failed", result.stdout)

    def test_target_summaries_must_match_their_targets_counts(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n◇ Test three() started.\n"
                               "✔ Test run with 3 tests passed after 0.1 seconds.\n"
                               "✔ Test run with 0 tests passed after 0.1 seconds.\n",
                               inventory=self.TWO_TARGETS)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("summaries report [0, 3] tests; the test targets declare [1, 2]", result.stderr)

    def test_zero_exit_with_failed_summary_still_fails(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n"
                               "✘ Test run with 2 tests failed after 0.1 seconds with 1 issue.\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("complete 2/2", result.stdout)

    def test_nonzero_exit_is_not_hidden_by_passing_summary(self):
        result = self.run_gate("◇ Test one() started.\n◇ Test two(_:) started.\n"
                               "✔ Test run with 2 tests passed after 0.1 seconds.\n", status=7)
        self.assertNotEqual(result.returncode, 0)

    def test_empty_or_malformed_discovery_cannot_pass(self):
        for inventory in ["", "build failed\n"]:
            with self.subTest(inventory=inventory):
                self.assertNotEqual(self.run_gate("", inventory=inventory).returncode, 0)


if __name__ == "__main__":
    unittest.main()
