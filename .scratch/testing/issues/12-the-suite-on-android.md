# 12 — The suite on Android

**What to build:** The core's tests running on the 64-bit ARM Android emulator. The Android SDK, NDK 27.1 and the `Medium_Phone` emulator (API 36.1) are already on this Mac.
1. Cross-compile the test binaries.
2. Push them to the emulator with the conformance data, using the data-folder switch from ticket 01 with live diffs off.
3. Run them there, including the stack tests at phone-sized thread stacks.

One command does all of this while an emulator is running. Then do one run on a real phone, if the maintainer has one; it stays the maintainer's phone, and the run changes nothing on it. This run isn't part of the check, because it needs an emulator, and hunts don't include it.

**Blocked by:** 01 — The suite runs anywhere

**Status:** ready-for-agent

- [ ] The maintainer approves installing cargo-ndk and the aarch64 Android Rust target before they're installed.
- [ ] One command runs the whole suite on a running emulator, and it passes.
- [ ] The stack tests pass at a 512 KB thread stack on the emulator.
- [ ] The real-phone run has passed, or this ticket records that there's no phone to run it on.
- [ ] TESTING.md has a section on Android.
- [ ] `scripts/check.sh` passes.
