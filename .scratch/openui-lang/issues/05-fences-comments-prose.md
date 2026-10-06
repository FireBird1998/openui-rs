# 05 — Fences, comments and prose around a program

**What to build:** The preprocessing the reference parser does before parsing: stripping markdown fences, comments, and prose before or after the program. Stage-level conformance against the reference package's exported `stripFences`, plus new corpus cases for the ways LLMs actually wrap output.

**Blocked by:** 02 — Real-world corpus.

**Status:** needs-triage

- [ ] Fence stripping matches the reference parser for every corpus program and prefix.
- [ ] New cases cover: fenced and unfenced programs, a fence that opens mid-stream, prose with apostrophes before the program, comment-like text inside strings, and trailing prose.
- [ ] Stripping arbitrary text never panics.
