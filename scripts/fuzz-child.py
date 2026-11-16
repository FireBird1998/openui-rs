#!/usr/bin/env python3
"""Bound one differential-fuzzer process tree; never install tools or inspect other jobs."""
import argparse
import ctypes
import json
import math
import os
import selectors
import signal
import struct
import subprocess
import sys
import time
from pathlib import Path

INPUT_BYTES = 64 * 1024
OUTPUT_BYTES = 4 * 1024 * 1024
STDERR_BYTES = 64 * 1024
POLL_SECONDS = 0.001
# Reserve an eighth for allocation between samples and process termination.
MEMORY_TRIGGER_NUMERATOR = 7
MEMORY_TRIGGER_DENOMINATOR = 8
LIBPROC = ctypes.CDLL("/usr/lib/libproc.dylib") if sys.platform == "darwin" else None


def memory_bytes(pid):
    if LIBPROC is not None:
        # rusage_info_v4: ri_phys_footprint at byte 72, including compression.
        buf = ctypes.create_string_buffer(4096)
        if LIBPROC.proc_pid_rusage(pid, 4, ctypes.byref(buf)) != 0:
            raise OSError("proc_pid_rusage unavailable")
        return struct.unpack_from("Q", buf, 72)[0]
    with open(f"/proc/{pid}/status", encoding="ascii") as status:
        sizes = {line.split(":", 1)[0]: int(line.split()[1]) * 1024
                 for line in status if line.startswith(("VmRSS:", "VmSwap:"))}
    # A zombie has no resident pages.
    return sizes.get("VmRSS", 0) + sizes.get("VmSwap", 0)


def owned_pids(leader):
    """Include the group after leader exit, and descendants in other groups."""
    if LIBPROC is not None:
        def listed(kind, ident):
            size = LIBPROC.proc_listpids(kind, ident, None, 0)
            if size <= 0:
                raise OSError("process enumeration unavailable")
            buf = (ctypes.c_int * (size // 4 + 32))()
            used = LIBPROC.proc_listpids(kind, ident, buf, ctypes.sizeof(buf))
            if used < 0:
                raise OSError("process enumeration unavailable")
            return {pid for pid in buf[:used // 4] if pid > 0}
        found = listed(2, leader)  # PROC_PGRP_ONLY
        pending = list(found | {leader})
        while pending:
            children = listed(6, pending.pop())  # PROC_PPID_ONLY
            pending.extend(children - found)
            found.update(children)
        return found
    table = {}
    for entry in Path("/proc").iterdir():
        if entry.name.isdecimal():
            try:
                # comm may itself contain parentheses and spaces.
                fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
                table[int(entry.name)] = (int(fields[1]), int(fields[2]))
            except (FileNotFoundError, ProcessLookupError):
                pass
    found = {pid for pid, (_, group) in table.items() if group == leader}
    found.add(leader)
    while True:
        children = {pid for pid, (parent, _) in table.items() if parent in found}
        if children <= found:
            return found & table.keys()
        found.update(children)


def group_memory_bytes(leader):
    total = 0
    for pid in owned_pids(leader):
        try:
            total += memory_bytes(pid)
        except OSError:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                continue
            raise
    return total


def kill_owned(leader):
    # Always attempt group cleanup, including when enumeration itself fails.
    try:
        pids = owned_pids(leader)
    except OSError:
        pids = set()
    try:
        for pid in pids:
            if pid != leader:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
    finally:
        try:
            os.killpg(leader, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            # A sandbox can deny killpg after the guard exited, while its
            # zombie is still listed. Individual owned PIDs remain killable.
            for pid in pids:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass


def watch_parent(fd, command):
    """Group leader holding the read end only: supervisor death produces EOF."""
    child = subprocess.Popen(command, close_fds=True)
    selector = selectors.DefaultSelector()
    selector.register(fd, selectors.EVENT_READ)
    try:
        while True:
            if selector.select(POLL_SECONDS) and not os.read(fd, 1):
                kill_owned(os.getpid())
                return 1
            status = child.poll()
            if status is not None and not (owned_pids(os.getpid()) - {os.getpid()}):
                return status if status >= 0 else 128 - status
    finally:
        selector.close()
        os.close(fd)


def supervise(command, request, seconds, memory_mb):
    if len(request) > INPUT_BYTES:
        return {"workerError": "input limit"}
    if not command or not math.isfinite(seconds) or seconds <= 0 or memory_mb <= 0:
        return {"workerError": "invalid supervisor limits or command"}
    # Capture ancestry before a worker can signal readiness or the hunt can
    # die; startup is part of the deadline too.
    started, parent = time.monotonic(), os.getppid()
    read_fd, alive_fd = os.pipe()
    try:
        child = subprocess.Popen(
            [sys.executable, str(Path(__file__).resolve()), "--guard", str(read_fd), "--", *command],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=True, pass_fds=(read_fd,))
    except BaseException:
        os.close(alive_fd)
        raise
    finally:
        os.close(read_fd)
    output, errors = bytearray(), bytearray()
    reason, peak = None, 0
    trigger = memory_mb * 1024 * 1024 * MEMORY_TRIGGER_NUMERATOR // MEMORY_TRIGGER_DENOMINATOR
    selector = selectors.DefaultSelector()
    try:
        for stream, label in [(child.stdin, "in"), (child.stdout, "out"), (child.stderr, "err")]:
            os.set_blocking(stream.fileno(), False)
            selector.register(stream, selectors.EVENT_WRITE if label == "in" else selectors.EVENT_READ, label)
        sent = 0
        while selector.get_map() or child.poll() is None:
            if os.getppid() != parent:
                reason = "parent exited"
                break
            if time.monotonic() - started >= seconds:
                reason = "deadline"
                break
            try:
                peak = max(peak, group_memory_bytes(child.pid))
                if peak > trigger:
                    reason = "memory limit"
                    break
            except OSError:
                reason = "memory measurement unavailable"
                break
            for key, _ in selector.select(timeout=POLL_SECONDS):
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
        # EOF also arms cleanup if this process dies during finalization.
        os.close(alive_fd)
        try:
            kill_owned(child.pid)
        finally:
            child.wait()
            selector.close()
            for stream in (child.stdin, child.stdout, child.stderr):
                stream.close()
    if reason or child.returncode != 0:
        return {"workerError": reason or f"exit {child.returncode}", "peakBytes": peak,
                "triggerBytes": trigger, "stderr": errors.decode("utf-8", errors="replace")[-4096:]}
    try:
        return json.loads(output)
    except (ValueError, UnicodeError):
        return {"workerError": "invalid worker JSON", "stderr": errors.decode("utf-8", errors="replace")[-4096:]}


def main():
    def interrupted(signum, _frame):
        raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, interrupted)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--memory-mb", type=int, default=256)
    parser.add_argument("--guard", type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if args.guard is not None:
        raise SystemExit(watch_parent(args.guard, command))
    try:
        result = supervise(command, sys.stdin.buffer.read(INPUT_BYTES + 1), args.seconds, args.memory_mb)
    except OSError as error:
        result = {"workerError": str(error)}
    print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))


if __name__ == "__main__":
    main()
