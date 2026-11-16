#!/usr/bin/env python3
"""Bound one differential-fuzzer child; never install tools or inspect other jobs."""
import argparse
import ctypes
import os
import selectors
import signal
import struct
import subprocess
import sys
import time
import json

INPUT_BYTES = 64 * 1024
OUTPUT_BYTES = 4 * 1024 * 1024
STDERR_BYTES = 64 * 1024


def memory_bytes(pid):
    if sys.platform == "darwin":
        # rusage_info_v4: ri_phys_footprint at byte 72, including compression.
        buf = ctypes.create_string_buffer(4096)
        lib = ctypes.CDLL("/usr/lib/libproc.dylib")
        if lib.proc_pid_rusage(pid, 4, ctypes.byref(buf)) != 0:
            raise OSError("proc_pid_rusage unavailable")
        return struct.unpack_from("Q", buf, 72)[0]
    with open(f"/proc/{pid}/status", encoding="ascii") as status:
        for line in status:
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) * 1024
    raise OSError("RSS unavailable")


def supervise(command, request, seconds, memory_mb):
    if len(request) > INPUT_BYTES:
        return {"workerError": "input limit"}
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, start_new_session=True)
    output, errors = bytearray(), bytearray()
    reason = None
    started = time.monotonic()
    selector = selectors.DefaultSelector()
    try:
        # Nonblocking input too: a child that never reads cannot stall the guard.
        for stream, label in [(child.stdin, "in"), (child.stdout, "out"), (child.stderr, "err")]:
            os.set_blocking(stream.fileno(), False)
            selector.register(stream, selectors.EVENT_WRITE if label == "in" else selectors.EVENT_READ, label)
        sent = 0
        while selector.get_map() or child.poll() is None:
            if time.monotonic() - started >= seconds:
                reason = "deadline"
                break
            if child.poll() is None:
                try:
                    if memory_bytes(child.pid) > memory_mb * 1024 * 1024:
                        reason = "memory limit"
                        break
                except OSError:
                    if child.poll() is None:
                        reason = "memory measurement unavailable"
                        break
            for key, _ in selector.select(timeout=0.01):
                stream, label = key.fileobj, key.data
                if label == "in":
                    try:
                        sent += os.write(stream.fileno(), request[sent:sent + 8192])
                    except BrokenPipeError:
                        sent = len(request)
                    if sent == len(request):
                        selector.unregister(stream)
                        stream.close()
                    continue
                block = os.read(stream.fileno(), 8192)
                if not block:
                    selector.unregister(stream)
                    stream.close()
                    continue
                target, limit = (output, OUTPUT_BYTES) if label == "out" else (errors, STDERR_BYTES)
                if len(target) + len(block) > limit:
                    reason = "output limit" if label == "out" else "stderr limit"
                    break
                target.extend(block)
            if reason:
                break
    finally:
        # Kill the owned process group even if its leader has already exited.
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()
        selector.close()
        for stream in (child.stdin, child.stdout, child.stderr):
            stream.close()
    if reason or child.returncode != 0:
        return {"workerError": reason or f"exit {child.returncode}",
                "stderr": errors.decode("utf-8", errors="replace")[-4096:]}
    try:
        return json.loads(output)
    except (ValueError, UnicodeError):
        return {"workerError": "invalid worker JSON", "stderr": errors.decode("utf-8", errors="replace")[-4096:]}


def main():
    def interrupted(signum, _frame):
        # Unwind supervise's finally block so an interrupted hunt reaps too.
        raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, interrupted)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--memory-mb", type=int, default=256)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    try:
        result = supervise(command, sys.stdin.buffer.read(INPUT_BYTES + 1), args.seconds, args.memory_mb)
    except OSError as error:
        result = {"workerError": str(error)}
    print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))


if __name__ == "__main__":
    main()
