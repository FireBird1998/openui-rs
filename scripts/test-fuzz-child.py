#!/usr/bin/env python3
"""Exercise actual owned children; elapsed time never decides a test assertion."""
import importlib.util
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location("fuzz_child", Path(__file__).with_name("fuzz-child.py"))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


def child(code, request=b"", seconds=2, memory_mb=256):
    return guard.supervise([sys.executable, "-c", code], request, seconds, memory_mb)


def main():
    assert child('import sys; print(sys.stdin.read())', b'{"ok":true}') == {"ok": True}
    assert child('print("oops")')["workerError"] == "invalid worker JSON"
    assert child('raise SystemExit(7)')["workerError"] == "exit 7"
    assert child('while True: pass', seconds=0.1)["workerError"] == "deadline"
    assert child('while True: pass', b"x" * 60000, seconds=0.1)["workerError"] == "deadline"
    assert child('import sys; sys.stdout.write("x" * 5000000)')["workerError"] == "output limit"
    assert child('import sys; sys.stderr.write("x" * 100000)')["workerError"] == "stderr limit"
    assert child('a=bytearray(64*1024*1024)\nwhile True: pass', memory_mb=32)["workerError"] == "memory limit"
    assert child('print("null")', b"x" * (guard.INPUT_BYTES + 1))["workerError"] == "input limit"
    measure = guard.memory_bytes
    try:
        def unavailable(_pid):
            raise OSError("fixture: memory measurement denied")
        guard.memory_bytes = unavailable
        assert child('while True: pass')["workerError"] == "memory measurement unavailable"
    finally:
        guard.memory_bytes = measure
    print("fuzz child: 10 checks passed (success, JSON, exit, deadline, blocked stdin, stdout, stderr, memory, input, unavailable measurement)")


if __name__ == "__main__":
    main()
