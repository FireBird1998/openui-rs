# 01 — One corpus case, drawn as boxes

**What to build:** The first path from the core to the screen. A binding crate wraps the core with UniFFI and exposes two things: loading a component library, and one-shot parsing, which returns the parse result as JSON text. One command builds the Rust library and generates the Swift bindings. The viewer's Xcode project, kept as text files in the repository, calls that same command, so building with ⌘R and building from the terminal give the same result. The viewer opens a corpus case picked from a plain list, parses it one-shot, decodes the JSON into its Swift model, and draws every element as a labeled box showing its type and props, with its children inside.

**Blocked by:** None — can start immediately.

**Status:** done (2026-10-15)

- [x] The binding crate is in the workspace, and the core crate gains no binding code.
- [x] The binding is built at opt-level 1 or more in every profile, debug included.
- [x] The Swift bindings are generated from inside the workspace, with no global tool installed for them.
- [x] For every corpus case, the binding's one-shot JSON equals the core's. This test runs in `scripts/check.sh`, which still needs only Rust.
- [x] One terminal command builds the binding and the viewer, and ⌘R in Xcode does the same.
- [x] The Xcode project can be regenerated from text files in the repository. If that needs a generator tool, the maintainer is asked before it's installed.
- [x] Opening a corpus case shows its elements as nested labeled boxes, each with its type and props.
- [x] An error from the core reaches Swift as a Swift error, never as a crash.
- [x] The viewer's Swift tests run from one command, in a check script separate from `scripts/check.sh`, and include a test that decodes one corpus case and draws it offscreen.
- [x] The README has a short viewer section on building and running it.
- [x] `scripts/check.sh` and the viewer's check pass before the commit.

## Comments

**2026-10-15, done.** The binding and the viewer both build. All checks pass: `scripts/check.sh` (the binding's 3 tests, including one-shot parsing against the core on every corpus case), `scripts/check-viewer.sh` (7 Swift tests) and a build of the binding on Rust 1.88.0. Looked at in the running app: `hand/01`, `hand/04` and upstream's `bench/dashboard` draw as nested boxes, each with its statement id.

- **No `.xcodeproj`.** The viewer is a Swift package, which Xcode opens directly. The package manifest and a shared scheme are plain text, so no generator tool was needed and nothing was installed. The scheme has a pre-action that runs `scripts/viewer.sh rust` before every Xcode build. Xcode doesn't show that action's output, so it goes to `target/viewer-xcode-prebuild.log`. Checked with `xcodebuild`: touching the binding's source rebuilt the library before the Swift build.
- **An app bundle.** `swift build` makes a bare executable, which has no bundle identifier, so it isn't a proper Mac app. The script wraps it in `target/viewer/<config>/Viewer.app` (bundle id `dev.openui-rs.viewer`, ad-hoc signed). Ticket 06's Keychain items will hang off that identity.
- **UniFFI 0.32.2** (MPL-2.0, noted in the README), with its default features off, so the library the app links carries no build-time tooling. The bindings generator is a binary in the binding crate, built with its own target directory so its features never touch the linked library. The static library and C header ship to Swift as an XCFramework. The generated Swift code already handles Swift 6 concurrency; it's compiled in Swift 5 mode anyway, to be safe.
- **What crosses the boundary:** the JSON from `ParseResult::to_json`, plus `depth_limited` and `size_limited` as separate fields, because the reference's JSON has no place for them.
- **Decoding in Swift:** a small JSON reader that keeps key order, so props appear in the core's order. JavaScript strings can hold lone surrogates that Swift strings can't, so those become U+FFFD.
- **A SwiftUI warning at launch:** "reentrant operation in its NSTableView delegate". It happens even with the viewer's own selection handling removed, so it comes from SwiftUI's sidebar list. Watch it in later tickets.
- **Mistake, since undone:** `cargo +1.88` made rustup install a second Rust 1.88 toolchain, named `1.88` next to the existing `1.88.0`. It was uninstalled right away. Use `+1.88.0`.
