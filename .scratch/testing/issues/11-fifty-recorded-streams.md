# 11 — Fifty recorded streams

**What to build:** Real model output in the corpus.
1. **The requests.** Claude writes 50 requests against upstream's example library, and they're committed with this ticket:
   - 40 fresh screens, about half of them news-shaped as a preview of Dynamic News;
   - 10 follow-ups to some of those screens, so merging gets real patches.
2. **Recording.** The maintainer records them in the viewer, with Claude Sonnet 5.5 and Claude Haiku 5.5 about half each, under the $10 budget cap.
3. **Importing.** Ticket 10's command imports them all.
4. **Findings.** Every one is sorted: mismatch, crash, blow-up or new quirk. Each is then fixed, or recorded with a draft upstream report.

**Blocked by:** 10 — Recordings become corpus cases

**Status:** ready-for-agent

- [ ] The 50 requests are committed.
- [ ] The maintainer has recorded them, and the estimated spend is reported here, within the cap.
- [ ] All 50 are imported. Every recorded case conforms, or its finding is resolved.
- [ ] What real output brought to light (fences, truncation, invented components, anything else) is summarized in this ticket.
- [ ] CONFORMANCE.md reports the recorded streams.
- [ ] `scripts/check.sh` passes.
