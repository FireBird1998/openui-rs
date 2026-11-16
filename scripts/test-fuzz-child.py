#!/usr/bin/env python3
"""Real owned children: bounded fixtures, outcome assertions, no elapsed-time assertions."""
import importlib.util
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("fuzz_child", Path(__file__).with_name("fuzz-child.py"))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)
# Every fixture exits independently, even if every supervisor check is broken.
# Every allocator below has a finite total <= 512 MiB, including descendants.
PREFIX = 'import os, signal, time; signal.alarm(15); end=time.monotonic()+12\n'
WAIT = '\nwhile time.monotonic()<end: time.sleep(.001)\n'


def child(code, request=b"", seconds=2, memory_mb=256):
    return guard.supervise([sys.executable, "-c", PREFIX + code], request, seconds, memory_mb)


class SupervisorTests(unittest.TestCase):
    def test_invalid_supervisor_limits_refuse_before_spawn(self):
        for seconds in [0, -1, float("nan"), float("inf")]:
            self.assertEqual(child('print("null")', seconds=seconds), {"workerError":"invalid supervisor limits or command"})
        for memory in [0, -1]:
            self.assertEqual(child('print("null")', memory_mb=memory), {"workerError":"invalid supervisor limits or command"})
        self.assertEqual(guard.supervise([],b"",2,256), {"workerError":"invalid supervisor limits or command"})

    def test_parent_change_during_spawn_is_observed(self):
        spawn, parent = guard.subprocess.Popen, guard.os.getppid
        started = False
        def spawning(*args, **kwargs):
            nonlocal started
            process = spawn(*args, **kwargs)
            started = True
            return process
        try:
            guard.os.getppid = lambda: 42 if started else 41
            guard.subprocess.Popen = spawning
            self.assertEqual(child(WAIT)["workerError"], "parent exited")
        finally:
            guard.subprocess.Popen = spawn
            guard.os.getppid = parent

    def test_success_preserves_worker_json(self):
        self.assertEqual(child('import sys; print(sys.stdin.read())', b'{"ok":true}'), {"ok": True})

    def test_invalid_json_is_a_finding(self):
        self.assertEqual(child('print("oops")')["workerError"], "invalid worker JSON")

    def test_nonzero_exit_is_a_finding(self):
        self.assertEqual(child('raise SystemExit(7)')["workerError"], "exit 7")
        self.assertEqual(child('os.kill(os.getpid(), signal.SIGTERM)')["workerError"], "exit 143")

    def test_deadline_stops_a_self_limited_child(self):
        self.assertEqual(child(WAIT, seconds=.1)["workerError"], "deadline")

    def test_blocked_stdin_cannot_stall_deadline(self):
        self.assertEqual(child(WAIT, b"x" * 60000, seconds=.1)["workerError"], "deadline")

    def test_stdout_is_bounded(self):
        self.assertEqual(child('import sys; sys.stdout.write("x" * 5000000)')["workerError"], "output limit")

    def test_stderr_is_bounded(self):
        self.assertEqual(child('import sys; sys.stderr.write("x" * 100000)')["workerError"], "stderr limit")

    def test_memory_stops_a_finite_allocator(self):
        self.assertEqual(child('a=bytearray(64*1024*1024)' + WAIT, memory_mb=32)["workerError"], "memory limit")

    def test_input_is_bounded_before_spawn(self):
        self.assertEqual(child('print("null")', b"x" * (guard.INPUT_BYTES + 1))["workerError"], "input limit")

    def test_unavailable_measurement_refuses_and_cleans_up(self):
        measure = guard.memory_bytes
        try:
            def unavailable(_pid):
                raise OSError("fixture: memory measurement denied")
            guard.memory_bytes = unavailable
            self.assertEqual(child(WAIT)["workerError"], "memory measurement unavailable")
        finally:
            guard.memory_bytes = measure

    def test_grandchild_footprint_is_counted(self):
        # Leader, child and grandchild: only the grandchild holds 96 MiB.
        code = 'pid=os.fork(); signal.alarm(15)\nif pid==0:\n pid=os.fork(); signal.alarm(15)\n if pid==0: a=bytearray(96*1024*1024)\n' + WAIT
        self.assertEqual(child(code, memory_mb=64)["workerError"], "memory limit")

    def test_group_footprints_are_summed(self):
        code = 'a=bytearray(32*1024*1024)\npid=os.fork(); signal.alarm(15)\nif pid==0: b=bytearray(32*1024*1024)\n' + WAIT
        self.assertEqual(child(code, memory_mb=64)["workerError"], "memory limit")

    def test_exited_leader_does_not_hide_descendant_memory(self):
        code = 'pid=os.fork(); signal.alarm(15)\nif pid: os._exit(0)\na=bytearray(96*1024*1024)\n' + WAIT
        self.assertEqual(child(code, memory_mb=64)["workerError"], "memory limit")

    def test_descendant_in_a_new_session_is_counted(self):
        code = 'pid=os.fork(); signal.alarm(15)\nif pid==0:\n os.setsid()\n a=bytearray(96*1024*1024)\n' + WAIT
        self.assertEqual(child(code, memory_mb=64)["workerError"], "memory limit")

    def test_reserve_reduces_finite_allocator_overshoot(self):
        # At most 512 MiB total, in touched 1 MiB blocks. This checks sampled
        # footprint, not a promise of a hard OS allocation ceiling.
        code = 'a=[]\nfor _ in range(512): a.append(bytearray(1024*1024))\n' + WAIT
        result = child(code)
        self.assertEqual(result["workerError"], "memory limit")
        self.assertLess(result["triggerBytes"], 256*1024*1024)
        self.assertLess(result["peakBytes"], 256*1024*1024)

    def test_sigkilled_supervisor_leaves_no_pipe_holding_descendants(self):
        self.check_supervisor_death(False)

    def test_sigkilled_supervisor_with_exited_worker_leaves_no_descendants(self):
        self.check_supervisor_death(True)

    def test_killed_hunt_parent_triggers_supervisor_cleanup(self):
        self.check_supervisor_death(False, kill_hunt=True)

    def check_supervisor_death(self, exit_leader, kill_hunt=False):
        with tempfile.TemporaryDirectory() as tmp:
            fifo = Path(tmp) / "liveness"
            os.mkfifo(fifo)
            fd = os.open(fifo, os.O_RDONLY | os.O_NONBLOCK)
            selector = selectors.DefaultSelector()
            selector.register(fd, selectors.EVENT_READ)
            # An outliving worker reports its independent exit instead of
            # silently closing: EOF can only pass after the guard killed it.
            setup = f'f=open({str(fifo)!r},"w"); pid=os.fork(); signal.alarm(15)\n'
            if exit_leader:
                setup += 'if pid: os._exit(0)\n'
            setup += 'if not pid: f.write("ready\\n"); f.flush()\n'
            code = PREFIX + setup + WAIT + 'f.write("self-exit"); f.flush()\n'
            command = [sys.executable, str(Path(guard.__file__))]
            if kill_hunt:
                # A broken parent watch must hit the fixture's own sentinel
                # before the ordinary supervisor deadline can mask the bug.
                command.extend(["--seconds", "18"])
            command.extend(["--", sys.executable, "-c", code])
            if kill_hunt:
                launcher = PREFIX + f'import subprocess; p=subprocess.Popen({command!r},stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n' + WAIT
                command = [sys.executable,"-c",launcher]
            supervisor = subprocess.Popen(command,
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                self.assertTrue(selector.select(10), "self-limited child never signalled readiness")
                self.assertEqual(os.read(fd, 100), b"ready\n")
                os.set_blocking(fd, True)
                supervisor.kill()
                supervisor.wait(timeout=10)
                self.assertEqual(os.read(fd, 100), b"")
            finally:
                if supervisor.poll() is None:
                    supervisor.kill()
                supervisor.wait(timeout=20)
                supervisor.stdout.close()
                supervisor.stderr.close()
                selector.close()
                os.close(fd)


if __name__ == "__main__":
    unittest.main()
