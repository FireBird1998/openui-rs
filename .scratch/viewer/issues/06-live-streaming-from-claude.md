# 06 — Live streaming from Claude

**What to build:** The full Dynamic News loop on the Mac. In the viewer's settings, the maintainer enters an Anthropic API key, which is kept in the macOS Keychain, and sets a budget cap. They type a request, pick a model (Claude Sonnet 5.5 or Claude Haiku 5.5) and a component library. The core generates the system prompt for that library; the binding gains prompt generation for this. The model's answer streams through a live source of chunks and draws as it arrives, with the same drawing and inspector as replays. Each stream shows its token usage and estimated cost, and the budget cap stops new requests once it's reached.

**Blocked by:** 03 — Corpus browser and inspector

**Status:** done (2026-10-15), except the maintainer's check with a real request

- [x] Model ids, the streaming event format and pricing are checked against the Claude API reference when the ticket starts.
- [x] The key lives only in the Keychain. It's never written to a file or a log, and is shown masked after entry. The maintainer enters it; no agent session handles it.
- [x] The binding's system prompt equals the core's for every case in the prompt corpus. This test runs in `scripts/check.sh`.
- [x] A live stream draws as it arrives, with the same drawing and inspector as a replay.
- [x] Each stream shows its token usage and estimated cost. Reaching the budget cap blocks new requests and says why.
- [x] Network errors, API errors and cancelling are shown without crashing, and cancelling stops the stream.
- [ ] The maintainer has checked it end to end with a real request. An agent can't do this step, because it needs the key.

## Comments

**2026-10-15, built and tested; one check left for the maintainer.** `scripts/check.sh` passes: the binding's prompts equal the core's for all 44 prompt specs. `scripts/check-viewer.sh` passes with 41 Swift tests. A stub `URLProtocol` serves canned server-sent event streams, so everything but the real network call is covered:
- **The request:** endpoint, headers, effort, `fallbacks`, and that the key travels only in `x-api-key`, never in the body or URL.
- **The event stream:** parsing it into text, usage, the stop reason, fallbacks and errors.
- **A stream end to end:** a whole stream arriving as chunks with its usage; a refusal; an HTTP 401 with the API's message; an error in mid-stream.
- **Playback:** following a source that drips in, and stopping it.
- **The prompt:** the system prompt for `bench` is exactly the reference's golden.

**Checked against the Claude API reference** (Anthropic's bundled API skill, cached 2026-10-06):
- `POST /v1/messages` with `stream: true`; text arrives as `content_block_delta` / `text_delta`.
- Input tokens arrive in `message_start`; output tokens arrive in `message_delta`, cumulatively, at the end.
- Model ids and prices per million tokens: `claude-opus-5-5` $4 / $20, `claude-sonnet-5-5` $2 / $10, `claude-haiku-5-5` $0.10 / $0.50.
- A refusal is `stop_reason: "refusal"`.

**How it works:**
- **Asking.** The toolbar's sparkles button opens "Ask Claude". It takes a request, a model (Opus 5.5 by default, as the reference recommends; Sonnet 5.5 and Haiku 5.5, which this spec named, are in the menu too), an effort (low, medium or high, default medium) and a component library. Only libraries with a prompt spec in the corpus are offered, and today that's `bench`.
- **The system prompt** comes from the core, through the binding's new `generate_system_prompt`.
- **The request** is raw HTTP, because there's no official Swift SDK:
  - `max_tokens` is 16,000, so one request costs at most about $0.32 on Opus;
  - the system prompt is cached (`cache_control` at the top level);
  - for Opus and Sonnet, it adds `fallbacks: "default"` under `server-side-fallback-2026-07-01`, as the reference recommends by default. Haiku 5.5 has no fallback.
- **Arriving.** Each text delta is one chunk. Playback now follows a source that's still arriving: it appends chunks and stays on the newest step unless the maintainer steps back. Stop cancels the HTTP request and keeps what arrived. The chunk menu is disabled while streaming.
- **The live bar** shows the phase (asking, thinking, writing, done, declined, stopped, failed), any fallback, the input and output tokens, and the estimated cost.
- **Cost.** It errs high on cache reads, and before output tokens arrive it counts text only. The estimated total is added to the spending in Settings. A request is refused once that reaches the cap, $10 by default, adjustable, with a reset.
- **The key** is entered in Settings, written straight to the login Keychain (service `dev.openui-rs.viewer`), and never shown again. The sheet only checks that a key exists; the key itself is read when a request starts. The viewer is ad-hoc signed, so macOS may ask to allow Keychain access after each rebuild.

**Left:** the maintainer adds a key in Settings and asks for one real screen. The agent can't do this.

Screen capture failed while this was being built, so the new sheet and settings haven't been looked at on screen, only built and exercised through tests.

**2026-10-15 (from ticket 08):** the sheet and Settings were looked at later, with the current build running. An old instance had been answering before: see ticket 08. The sheet was correct. Settings showed field placeholders as stray labels, now fixed.

