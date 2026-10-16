# 07 — Stream end

**What to build:** Ending a stream means what it means in 1.0, everywhere the core streams:
- **The `$name` rule:** until the stream ends, a `$name` at the very end of the text isn't declared as state (it may still be growing), unless the same name already appears in full in the pending text. `finish()` lifts it.
- **Streaming equals parsing:** the check that a stream ends where one-shot parsing ends compares the stream-end step now. Its exceptions are reviewed against 1.0. One candidate quirk: the reference skips a pending redefinition that needed closing even at stream end, which the spec says should win.
- **The binding and the viewer:** the binding's `StreamSession` gets `finish()`, and the viewer calls it when a stream ends.

**Blocked by:** 02 — Vendor lang-core 1.0.1.

**Status:** done (2026-10-16)

- [x] Every pending case whose difference is the stream end conforms.
- [x] The stream-end exceptions are reviewed, and any quirk is written up for review.
- [x] The binding's and the viewer's tests cover `finish()`.
- [x] `scripts/check.sh` and `scripts/check-viewer.sh` pass.

## Comments

**2026-10-16, done.**
- **The `$name` rule:** until the stream ends, the `$name` that ends the raw text (`/\$\w+$/`) isn't auto-declared as state by the pending statements it's in, when the pending text ends with it and doesn't already have the same name in full before it. A completed statement that uses it still declares it. `StreamParser::finish()` lifts the rule. `push`, `set` and `result` keep it.
- **Everywhere the core streams:** the binding's `StreamSession` has `finish()`, tested against the core on every case. The CLI's `stream` ends its output with the stream's end. In the viewer, once every chunk has arrived, the last step is the stream's end, and its golden check reads the golden's end step. A live stream, a recording and a corpus case alike end that way.

**The gap:** every parse case conforms, under every chunking and at every stream's end. What's left is ticket 08's: 43 trees back to source and 29 prompts.

**The stream-end exceptions**, where the reference's own stream end differs from its one-shot parse, are 23 cases of three kinds:
- 21 count a redefinition twice (quirk 4);
- `hand/71` has a mismatched closing bracket (quirk 5);
- `hand/85` ends on a redefinition that needed closing, which the stream keeps out (new quirk 11).

The 1.0 spec says a stream's end must equal a batch parse (language.md, section 4) and that a final redefinition wins at the end (section 2.4). All three are written up in QUIRKS.md with draft upstream reports, for the maintainer to decide on.

**Tests:** the binding checks `finish()` on every case. The viewer checks that the last step is the stream's end (a program ending in `$na`), and that an ended stream's last step is checked against the golden's end.
