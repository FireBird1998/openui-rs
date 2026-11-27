#!/usr/bin/env python3
"""Audit the executable's actual static linker inputs and Mach-O load commands."""

import json
from pathlib import Path
import plistlib
import re
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parent.parent
PACKAGE = ROOT / "tools/chat"
LINKED = {
    "MCP": "swift-sdk",
    "SystemPackage": "swift-system",
    "CSystem": "swift-system",
    "Logging": "swift-log",
    "EventSource": "eventsource",
}
RESOLUTION_ONLY = {
    "swift-atomics", "swift-collections", "swift-docc-plugin",
    "swift-docc-symbolkit", "swift-nio",
}
LOCAL = {"Chat", "ChatKit", "OpenUINative", "OpenUIFFI"}


def check_pins():
    pins = json.loads((PACKAGE / "Package.resolved").read_text())["pins"]
    identities = {pin["identity"] for pin in pins}
    required = set(LINKED.values())
    if not required <= identities or identities - required - RESOLUTION_ONLY:
        raise ValueError(f"unexpected resolved packages: {sorted(identities)}")


def check_map(path):
    sdk = Path(subprocess.check_output(["xcrun", "--sdk", "macosx", "--show-sdk-path"], text=True).strip())
    swift = Path(subprocess.check_output(["xcrun", "--find", "swift"], text=True).strip())
    swift_libraries = swift.parent.parent / "lib" / "swift"
    executable = None
    modules = set()
    in_objects = False
    with path.open() as lines:
        for line in lines:
            if line.startswith("# Path: "):
                executable = line.removeprefix("# Path: ").strip()
            elif line.startswith("# Object files:"):
                in_objects = True
            elif in_objects and line.startswith("#"):
                break
            elif in_objects:
                match = re.fullmatch(r"\[\s*\d+\]\s+(.+)\n", line)
                if not match:
                    raise ValueError(f"unexpected link map entry: {line.strip()}")
                obj = match.group(1)
                source = Path(obj.split("(", 1)[0])
                if obj == "linker synthesized" or source.name == "libopenui_ffi.a":
                    continue
                if source.suffix == ".tbd" and source.is_relative_to(sdk):
                    continue
                if source.is_relative_to(swift_libraries) and re.fullmatch(r"libswiftCompatibility\w+\.a", source.name):
                    continue
                name = source.stem
                if name not in LINKED and name not in LOCAL:
                    # SwiftPM's native build links individual .o files from
                    # module.build. Its Xcode build links each merged module.
                    name = next(
                        (part.removesuffix(".build").removesuffix("-p").removesuffix("-t")
                         for part in reversed(source.parts) if part != ".build" and part.endswith(".build")),
                        name,
                    )
                if name not in LINKED and name not in LOCAL:
                    raise ValueError(f"unapproved Chat linker input: {obj}")
                modules.add(name)
    if not executable or not LINKED.keys() <= modules:
        raise ValueError(f"missing executable or approved modules in link map: {sorted(modules)}")
    linked = subprocess.check_output(["otool", "-L", executable], text=True).splitlines()[1:]
    for line in linked:
        dependency = line.strip().split(" (", 1)[0]
        if not dependency.startswith(("/System/Library/", "/usr/lib/")):
            raise ValueError(f"non-system dynamic library linked by Chat: {dependency}")


def main():
    check_pins()
    build = PACKAGE / ".build"
    build.mkdir(exist_ok=True)
    with TemporaryDirectory(prefix="check-chat-link-", dir=build) as temporary:
        link_map = Path(temporary) / "Chat.map"
        subprocess.run(
            ["swift", "build", "--package-path", str(PACKAGE), "--product", "Chat",
             *sys.argv[1:], "-Xlinker", "-map", "-Xlinker", str(link_map)],
            check=True,
        )
        check_map(link_map)
    entitlements = plistlib.loads((PACKAGE / "App/Chat.entitlements").read_bytes())
    expected = {"com.apple.security." + key for key in ("app-sandbox", "network.client", "network.server")}
    if entitlements != dict.fromkeys(expected, True):
        raise ValueError("unexpected Chat sandbox entitlements")
    print("Chat link and entitlement audit passed: MCP, swift-system, swift-log, eventsource and system libraries")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(f"check-chat-linkage: {error}")
