#!/usr/bin/env python3
"""Regenerate only the six #69 fuzz pins, offline, under the worker supervisor."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("fuzz_child", ROOT / "scripts/fuzz-child.py")
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="refuse a changed pin instead of writing")
    args = parser.parse_args()
    env = dict(os.environ, CARGO_NET_OFFLINE="true", CARGO_TARGET_DIR=str(ROOT / "target"))
    subprocess.run(["cargo", "build", "--offline", "-q", "-p", "openui-lang", "--example", "fuzz"],
                   cwd=ROOT, env=env, check=True)
    pins = []
    for index in range(6):
        os.chdir(ROOT)
        case = guard.supervise([str(ROOT / "target/debug/examples/fuzz"), "--runtime-case",
                                "--seed", "69", "--only", str(index)], b"", 2, 256)
        if "workerError" in case:
            raise RuntimeError(case)
        os.chdir(ROOT / "reference")
        reference = guard.supervise(["bun", "--no-install", "runtime.ts", "fuzz"],
                                    json.dumps(case, ensure_ascii=False).encode(), 2, 256)
        if "workerError" in reference:
            raise RuntimeError(reference)
        pins.append({"case": case, "reference": reference})
    path = ROOT / "crates/openui-lang/examples/fuzz/runtime/pins.json"
    encoded = json.dumps(pins, ensure_ascii=False, separators=(",", ":")) + "\n"
    if args.check:
        if path.read_text() != encoded:
            raise SystemExit("runtime fuzz pins differ; run scripts/pin-runtime-fuzz.py and review")
        print("runtime fuzz: six pins unchanged")
    else:
        temporary = path.with_suffix(".json.tmp")
        temporary.write_text(encoded)
        temporary.replace(path)
        print("runtime fuzz: six pins regenerated")


if __name__ == "__main__":
    main()
