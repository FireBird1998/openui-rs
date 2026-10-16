# 07 — Stream end

**What to build:** Ending a stream means what it means in 1.0, everywhere the core streams:
- **The `$name` rule:** until the stream ends, a `$name` at the very end of the text isn't declared as state (it may still be growing), unless the same name already appears in full in the pending text. `finish()` lifts it.
- **Streaming equals parsing:** the check that a stream ends where one-shot parsing ends compares the stream-end step now. Its exceptions are reviewed against 1.0. One candidate quirk: the reference skips a pending redefinition that needed closing even at stream end, which the spec says should win.
- **The binding and the viewer:** the binding's `StreamSession` gets `finish()`, and the viewer calls it when a stream ends.

**Blocked by:** 02 — Vendor lang-core 1.0.1.

**Status:** ready-for-agent

- [ ] Every pending case whose difference is the stream end conforms.
- [ ] The stream-end exceptions are reviewed, and any quirk is written up for review.
- [ ] The binding's and the viewer's tests cover `finish()`.
- [ ] `scripts/check.sh` and `scripts/check-viewer.sh` pass.
