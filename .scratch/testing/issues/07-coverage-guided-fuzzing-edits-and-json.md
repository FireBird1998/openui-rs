# 07 — Coverage-guided fuzzing: edits and JSON

**What to build:** New coverage-guided targets for merging, converting element trees back to source, and reading JSON. Each runs a full night, the same way as ticket 06, and is added to hunts.

**Blocked by:** 06 — Coverage-guided fuzzing: parsing

**Status:** ready-for-agent

- [ ] Three new targets, each starting from the matching corpus: the merge cases, the serialization cases, and the JSON the corpus holds.
- [ ] Each has run a full night, with its numbers recorded, and nothing new turned up by the end.
- [ ] Every finding is a case, fixed or explained.
- [ ] Hunts run these targets.
- [ ] `scripts/check.sh` passes.
