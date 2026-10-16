# 09 — Finish line

**What to build:** The core conforms to lang-core 1.0.1 with nothing pending, and everything around it says so:
- every pending list is empty, and CONFORMANCE.md names 1.0.1 with its counts;
- each divergence and quirk is reviewed against 1.0: removed if it's gone, rewritten if it changed, and new ones added with draft upstream reports for the maintainer;
- the fuzzer's generator writes calls to library functions and actions, and uses 1.0 libraries;
- the viewer uses 1.0's react-ui library spec, so its system prompts are 1.0's;
- a hunt reports no lang-core commits since the pin, and no findings.

**Blocked by:** 04 — The entry rule; 05 — Calls; 06 — Validation; 07 — Stream end; 08 — Converting back to source, the prompt and error hints.

**Status:** ready-for-agent

- [ ] Every pending list is empty.
- [ ] CONFORMANCE.md, QUIRKS.md, TESTING.md and the README describe 1.0.
- [ ] A hunt of at least ten minutes finds nothing and reports the pin current.
- [ ] `scripts/check.sh` and `scripts/check-viewer.sh` pass.
