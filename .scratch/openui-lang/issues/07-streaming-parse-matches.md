# 07 — Streaming parse matches at every step

**What to build:** The streaming parser (push, resync with the full text, current result), ported faithfully from the reference algorithm. Every corpus case conforms at every step of every chunking.

**Blocked by:** 06 — One-shot parse matches.

**Status:** ready-for-agent

- [ ] Every streaming step of every chunking of every corpus case conforms, and the pending list is empty.
- [ ] Resync cases conform: text that continues the old text, and text that doesn't (which resets).
- [ ] Redefined statement ids behave like the reference parser while streaming (the upstream #1127 scenario has a named case).
- [ ] `openui-lang stream <schema> <file> [chunk]` prints one result per push.

## Comments

**From ticket 01:** the streaming parser is already a faithful port of the reference algorithm (push only; `set` and the current-result accessor are still to add). A difference worth a named case: streaming `statementCount` counts re-definitions (`completed_count`), while one-shot counts distinct ids.

**From ticket 02:** while cases are pending, each stream stops at its first mismatch, so the suite takes about 1 s. Once everything conforms it will run every push: about 160k pushes over growing buffers, in a debug build. Time it when this ticket closes. If it's slow, raise `opt-level` for the test profile rather than trimming the corpus.

**From ticket 06:** streaming already conforms at every step of every chunking for all 284 cases, because the stream parser was a faithful port from ticket 01. The full conformance test takes about 7 s (opt-level 1). What's left here: `set` (resync and reset) and the current-result accessor, with cases; a named case for the upstream #1127 redefinition scenario (the reference tests the corpus imported may already cover it); and `openui-lang stream`.
