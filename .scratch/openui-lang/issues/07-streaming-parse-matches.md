# 07 — Streaming parse matches at every step

**What to build:** The streaming parser (push, resync with the full text, current result), ported faithfully from the reference algorithm. Every corpus case conforms at every step of every chunking.

**Blocked by:** 06 — One-shot parse matches.

**Status:** done (2026-10-06)

- [x] Every streaming step of every chunking of every corpus case conforms, and the pending list is empty.
- [x] Resync cases conform: text that continues the old text, and text that doesn't (which resets).
- [x] Redefined statement ids behave like the reference parser while streaming (the upstream #1127 scenario has a named case).
- [x] `openui-lang stream <schema> <file> [chunk]` prints one result per push.

## Comments

**From ticket 01:** the streaming parser is already a faithful port of the reference algorithm (push only; `set` and the current-result accessor are still to add). A difference worth a named case: streaming `statementCount` counts re-definitions (`completed_count`), while one-shot counts distinct ids.

**From ticket 02:** while cases are pending, each stream stops at its first mismatch, so the suite takes about 1 s. Once everything conforms it will run every push: about 160k pushes over growing buffers, in a debug build. Time it when this ticket closes. If it's slow, raise `opt-level` for the test profile rather than trimming the corpus.

**From ticket 06:** streaming already conforms at every step of every chunking for all 284 cases, because the stream parser was a faithful port from ticket 01. The full conformance test takes about 7 s (opt-level 1). What's left here: `set` (resync and reset) and the current-result accessor, with cases; a named case for the upstream #1127 redefinition scenario (the reference tests the corpus imported may already cover it); and `openui-lang stream`.

**2026-10-06, done.**
- `StreamParser::set(full_text)` (a continuation parses only the new part; anything else resets) and `StreamParser::result()` (getResult).
- Two new chunkings, `resync-1` and `resync-2`: seeded sequences of `set` with the program's first n chars, mostly growing but sometimes rewinding (reset) or switching to an edited copy whose middle char differs (reset once past the middle), then a final `result()`. The ops are stored in the golden (`ops: [["p"|"x", n] ..., ["r", 0]]`). Every case runs them, so both `set` branches run on all 285 cases. All conform.
- New `hand/85-streaming-redefinition-1127`: redefinitions arriving mid-stream, plus a pending redefinition that must not clobber a completed statement. It conforms under all 10 chunkings. The reference tests' own last-wins cases were already in the corpus (test/parser-*).
- Deliberate breaks: a reset that keeps the statement cache is caught at the first rewind. A reset that only clears the text can't be caught, because `refresh_cleaned` already discards a cache whose completed prefix no longer matches, so the reference's explicit reset is redundant there.
- `openui-lang stream <schema> <program> [chunk]`, tested against the golden steps.
- Quirk 4 recorded: streaming `statementCount` counts redefinitions, one-shot doesn't.
- The full conformance test takes about 6 s at opt-level 1, so no further tuning is needed (see the ticket 02 note). Goldens are now about 6 MB of files.
