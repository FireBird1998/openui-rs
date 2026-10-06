# 05 — Fences, comments and prose around a program

**What to build:** The preprocessing the reference parser does before parsing: stripping markdown fences, comments, and prose before or after the program. Stage-level conformance against the reference package's exported `stripFences`, plus new corpus cases for the ways LLMs actually wrap output.

**Blocked by:** 02 — Real-world corpus.

**Status:** ready-for-agent

- [ ] Fence stripping matches the reference parser for every corpus program and prefix.
- [ ] New cases cover: fenced and unfenced programs, a fence that opens mid-stream, prose with apostrophes before the program, comment-like text inside strings, and trailing prose.
- [ ] Stripping arbitrary text never panics.

## Comments

**From ticket 01:** preprocess is currently JS `trim` only (`js::trim` reproduces JS's whitespace set: it includes U+FEFF and excludes U+0085). `stripComments` carries its in-string state across lines, so a quote opened on one line affects comment detection on the next. Port that faithfully.
