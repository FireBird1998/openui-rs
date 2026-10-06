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
