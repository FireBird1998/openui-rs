#!/usr/bin/env python3
"""Exercise watchdog attribution and real kills using only owned fixture processes.

No cargo-mutants run is involved. A ps shim supplies the fixture process only,
so this test cannot discover or kill another session's work. On macOS the
footprint shim reads the same compressed-memory-inclusive kernel accounting
via libproc, including when the sandbox forbids the footprint executable.
"""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time


def executable(path, text):
    path.write_text(text)
    path.chmod(0o755)


def main():
    script = Path(__file__).with_name("mutants-watchdog.sh").resolve()
    with tempfile.TemporaryDirectory(prefix="watchdog-test-") as tmp:
        root = Path(tmp)
        build = root / "cargo-mutants-fixture-owned.tmp"
        binary = build / "target/debug/hog"
        binary.parent.mkdir(parents=True)
        source = root / "hog.c"
        source.write_text("#include <stdlib.h>\n#include <string.h>\n#include <unistd.h>\n"
                          "int main(void) { volatile char *p = malloc(64*1024*1024); "
                          "if (!p) return 1; for (int i=0;i<64*1024*1024;i+=4096) p[i]=1; "
                          "sleep(10); return p[0]; }\n")
        subprocess.run(["cc", str(source), "-o", str(binary)], check=True)
        output = root / "output/parsing/mutants.out"
        output.mkdir(parents=True)
        debug = output / "debug.log"
        debug.write_text(
            f'worker thread{{build_dir="{build}"}}:mutant{{name="old mutant"}}: start\n'
            f'worker thread{{build_dir="{build}"}}:mutant{{name="replace guard with false in loose_eq"}}: start\n'
            'worker thread{build_dir="/other/cargo-mutants-unrelated.tmp"}:'
            'mutant{name="unrelated mutant"}: start\n')
        (output / "lock.json").write_text('{"cargo_mutants_version":"27.1.0"}\n')
        original = debug.read_bytes()
        shims = root / "bin"
        shims.mkdir()
        executable(shims / "ps", '#!/bin/sh\nprintf "%s 0 %s\\n" "$FIXTURE_PID" "$FIXTURE_BINARY"\n')
        executable(shims / "awk", '''#!/usr/bin/env python3
import os, sys, time
from pathlib import Path
if any(arg.startswith('build=') for arg in sys.argv[1:]):
 deadline = time.monotonic() + 0.5
 while True:
  try: os.kill(int(os.environ['FIXTURE_PID']), 0)
  except ProcessLookupError: break
  if time.monotonic() >= deadline:
   Path(os.environ['ORDER_FAILURE']).touch()
   break
  time.sleep(0.01)
os.execv('/usr/bin/awk', ['awk'] + sys.argv[1:])
''')
        if os.uname().sysname == "Darwin":
            executable(shims / "footprint", '''#!/usr/bin/env python3
import ctypes, os, struct
pid=int(os.environ['FIXTURE_PID'])
buf=ctypes.create_string_buffer(4096)
lib=ctypes.CDLL('/usr/lib/libproc.dylib')
if lib.proc_pid_rusage(pid, 4, ctypes.byref(buf)) == 0:
 print('hog [%d]: 64-bit    Footprint: %d B' % (pid,struct.unpack_from('Q',buf,72)[0]))
''')
        for available in [True, False]:
            if not available:
                debug.write_text("")
                original = b""
            hog = subprocess.Popen([str(binary)])
            watchdog = None
            try:
                env = dict(os.environ, PATH=str(shims) + os.pathsep + os.environ["PATH"],
                           FIXTURE_PID=str(hog.pid), FIXTURE_BINARY=str(binary),
                           ORDER_FAILURE=str(root / "read-before-kill"),
                           MUTANTS_OUTPUT_ROOT=str(root / "output"),
                           MUTANTS_WATCHDOG_BUILD_ROOT=str(build), MUTANTS_MEMORY_LIMIT_MB="16")
                watchdog = subprocess.Popen([str(script), str(os.getpid())], env=env,
                                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
                deadline = time.monotonic() + 8
                while hog.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.02)
                assert hog.poll() == -9, "watchdog did not kill its fixture hog"
                time.sleep(0.1)  # let the kill diagnostic finish before shutdown
            finally:
                if hog.poll() is None:
                    hog.kill()
                hog.wait()
                if watchdog is not None:
                    watchdog.terminate()
                    _, log = watchdog.communicate(timeout=3)
            assert not (root / "read-before-kill").exists(), "attribution ran before kill"
            assert str(build) in log, log
            assert re.search(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", log), log
            assert ("replace guard with false in loose_eq" if available else "description unavailable") in log, log
            assert "unrelated mutant" not in log, log
            assert debug.read_bytes() == original, "watchdog modified cargo-mutants output"
            print(log.strip())
    print("watchdog fixture checks passed")


if __name__ == "__main__":
    main()
