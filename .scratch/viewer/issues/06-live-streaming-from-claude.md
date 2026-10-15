# 06 — Live streaming from Claude

**What to build:** The full Dynamic News loop on the Mac. In the viewer's settings, the maintainer enters an Anthropic API key, which is kept in the macOS Keychain, and sets a budget cap. They type a request, pick a model (Claude Sonnet 5.5 or Claude Haiku 5.5) and a component library. The core generates the system prompt for that library; the binding gains prompt generation for this. The model's answer streams through a live source of chunks and draws as it arrives, with the same drawing and inspector as replays. Each stream shows its token usage and estimated cost, and the budget cap stops new requests once it's reached.

**Blocked by:** 03 — Corpus browser and inspector

**Status:** ready-for-agent

- [ ] Model ids, the streaming event format and pricing are checked against the Claude API reference when the ticket starts.
- [ ] The key lives only in the Keychain. It's never written to a file or a log, and is shown masked after entry. The maintainer enters it; no agent session handles it.
- [ ] The binding's system prompt equals the core's for every case in the prompt corpus. This test runs in `scripts/check.sh`.
- [ ] A live stream draws as it arrives, with the same drawing and inspector as a replay.
- [ ] Each stream shows its token usage and estimated cost. Reaching the budget cap blocks new requests and says why.
- [ ] Network errors, API errors and cancelling are shown without crashing, and cancelling stops the stream.
- [ ] The maintainer has checked it end to end with a real request. An agent can't do this step, because it needs the key.
