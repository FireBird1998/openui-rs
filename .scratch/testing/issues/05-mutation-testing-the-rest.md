# 05 — Mutation testing: the rest of the crate

**What to build:** The same as ticket 04, for the rest of the core: merging, conversion back to source, prompt generation, error hints, JSON reading and writing, the component library, builtins, and the crate's front page. Add the binding crate too, whose tests check it against the core.

**Blocked by:** 04 — Mutation testing: parsing and streaming

**Status:** ready-for-agent

- [ ] The run's command, duration and counts are recorded in this ticket.
- [ ] Every missed mutant is caught by a new case or explained in the exclusion file.
- [ ] A rerun misses nothing unexplained.
- [ ] `scripts/check.sh` passes.
