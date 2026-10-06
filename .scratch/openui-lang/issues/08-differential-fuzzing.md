# 08 — Differential fuzzing against the reference

**What to build:** An opt-in fuzzing command. It generates random programs from the grammar and mutates them (truncate, delete or insert characters, swap tokens, wrap in fences or prose). Each program goes through the core and the reference parser under random chunkings, with a single long-lived Bun process answering batches. Mismatches are shrunk to a minimal program and saved as corpus cases with golden output. Behaviour that looks wrong in the reference parser goes on a quirks list with a draft upstream report.

**Blocked by:** 07 — Streaming parse matches.

**Status:** needs-triage

- [ ] A fuzz run of a chosen size and seed is reproducible from its seed.
- [ ] Each mismatch is shrunk and saved as a corpus case; the normal test run then covers it without Bun.
- [ ] A quirks document exists, and each entry has a minimal program, the reference behaviour and a draft upstream issue.
- [ ] A 10,000-program run finishes with no unexplained mismatches.
