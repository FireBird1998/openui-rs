# 12 — The suite on Android

**What to build:** The core's tests running on the 64-bit ARM Android emulator. The Android SDK, NDK 27.1 and the `Medium_Phone` emulator (API 36.1) are already on this Mac.
1. Cross-compile the test binaries.
2. Push them to the emulator with the conformance data, using the data-folder switch from ticket 01 with live diffs off.
3. Run them there, including the stack tests at phone-sized thread stacks.

One command does all of this while an emulator is running. Then do one run on a real phone, if the maintainer has one; it stays the maintainer's phone, and the run changes nothing on it. This run isn't part of the check, because it needs an emulator, and hunts don't include it.

**Blocked by:** 01 — The suite runs anywhere

**Status:** done (2026-11-11)

- [x] The maintainer approves installing cargo-ndk and the aarch64 Android Rust target before they're installed.
- [x] One command runs the whole suite on a running emulator, and it passes.
- [x] The stack tests pass at a 512 KB thread stack on the emulator.
- [x] The real-phone run has passed, or this ticket records that there's no phone to run it on. (No phone run: the maintainer's decision, 2026-11-11, is that the emulator is enough.)
- [x] TESTING.md has a section on Android.
- [x] `scripts/check.sh` passes.

## Results

**2026-11-06, the worker Mac's emulator:**
- Setup: `openui-arm64`, Android 16 (API 36, Google APIs, arm64-v8a), headless, NDK 27.1.12297006, cargo-ndk 4.1.2. The maintainer approved installing these on the worker Mac.
- Result: `scripts/android-test.sh` ran 12 test binaries, 55 tests, none failed, in 30 s. That includes the 11 depth tests at a 512 KB stack. `report` and `cli` stay on the host.
- Fixed on the way, in the commit that records this:
  - With cargo-ndk 4, the script found no test binaries ("nothing built"), here and in GitHub's x86_64 emulator run 41998291085. cargo-ndk 4 reads cargo's JSON messages itself and doesn't pass them on. The script now builds with `cargo ndk-test`, which does pass them on.
  - Two unit tests read the conformance folder from the source tree's path. They now honour `OPENUI_CONFORMANCE_DIR`.

**Still open:** the real-phone run, if the maintainer has a phone, or a note here that there isn't one. GitHub's x86_64 run needs repeating after a push.

**2026-11-09, the worker Mac's emulator again, after the hardening phase (GitHub #1):**
- Setup: `emulator-5572`, Android 16 arm64, another session's running emulator, used with the maintainer's OK. `scripts/android-test.sh` writes only under `/data/local/tmp/openui-rs`.
- **Before the fix:** 21 test binaries, 3 failed.
  - `drop`: the raised-depth case segfaulted at a 512 KB, 2 MB and 8 MB stack alike. That was a real core bug: `stacker` can't measure the stack on Android or iOS, so the core never grew it (GitHub #25).
  - `api` and `fuzz-example`: two tests read files from the source tree's path instead of `OPENUI_CONFORMANCE_DIR`.
- **After the fix:** 21 test binaries, none failed.
  - `drop` also passes at a 64 KB stack.
  - `depth` passes at 128 KB and up. At 64 KB, one of its tests overflows dropping its own 900-level `serde_json` value, which is test code and not the core.
- **Cost:** see ADR 0002.

**2026-11-11, no real-phone run.** The maintainer decided the 64-bit ARM emulator is enough: both Macs have Android Studio, and the emulator runs the same arm64 binaries a phone would. The suite also passes inside an iPhone 16 simulator (iOS 18.6) at a 512 KB stack, 22 test binaries, noted on #25.
