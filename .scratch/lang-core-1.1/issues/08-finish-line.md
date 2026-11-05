# 08 — Finish line

**What to build:** The core conforms to 1.1.0, and the long tests have run against it:
- nothing is pending;
- CONFORMANCE.md names 1.1 with the corpus counts;
- QUIRKS.md and the divergences are reviewed;
- the fuzzer's generators emit 1.1's new inputs;
- the viewer uses 1.1's react-ui spec.

On the worker Mac, against 1.1:
- the million-program invariants run;
- a full-length coverage-guided fuzzing run per target;
- mutation testing of both groups.

Their findings are sorted. The candidate quirks are drafted for the maintainer's judgement.

**Blocked by:** 02, 03, 04, 05, 06, 07

**Status:** ready-for-agent

- [ ] Every list in `conformance/pending/` is empty, and CONFORMANCE.md names 1.1.
- [ ] QUIRKS.md marks the quirks 1.1 fixed, and the rest are re-checked.
- [ ] The fuzzer generates comments, `openui-js` blocks, expression defaults and queries reading queries.
- [ ] The viewer's react-ui prompt comes from 1.1.
- [ ] A hunt reports the pin current.
- [ ] The long runs have run on the worker Mac against 1.1, their findings sorted into cases, exclusions or drafts for the maintainer.
- [ ] `scripts/check.sh` and `scripts/check-viewer.sh` pass.
