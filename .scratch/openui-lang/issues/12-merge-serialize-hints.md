# 12 — Edit-mode merge, JSON-to-OpenUI, and error hints match

**What to build:** The rest of the reference package's language layer:
- merging a patch program into an existing one (edit mode, including garbage-collecting unreachable statements)
- turning a JSON element tree back into OpenUI Lang
- enriching validation errors with signature hints

Each is conformant against the reference parser, using inputs lifted from its merge and serialize tests.

**Blocked by:** 06 — One-shot parse matches.

**Status:** ready-for-agent

- [ ] Merge output matches the reference for every case from its tests plus generated patch cases.
- [ ] JSON-to-OpenUI output matches the reference for every case from its serialize tests.
- [ ] Enriched errors match the reference for every validation case in the corpus.
