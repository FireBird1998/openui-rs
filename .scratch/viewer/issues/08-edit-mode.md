# 08 — Edit mode

**What to build:** On a screen that came from a live stream or a recording, the maintainer can send a follow-up request. The model gets the edit-mode system prompt, which the core generates with its edit-mode option, and answers with a patch. As the patch streams in, the core merges it into the current program (the binding gains merging), and the viewer redraws the merged program. The patch is shown beside the merged program. Follow-ups can be recorded. A follow-up's recording includes the program it patched, so replaying it reproduces the merge.

**Blocked by:** 06 — Live streaming from Claude; 07 — Recording and replay

**Status:** ready-for-agent

- [ ] The binding's merge equals the core's for every case in the merge corpus. This test runs in `scripts/check.sh`.
- [ ] Follow-ups use the core's edit-mode system prompt.
- [ ] The merged program redraws as the patch streams in.
- [ ] The patch is shown beside the merged program.
- [ ] A follow-up's recording includes the program it patched, and replaying it reproduces the same merged program at every step. A Swift test checks this with a hand-made recording.
- [ ] The maintainer has checked one real follow-up end to end.
