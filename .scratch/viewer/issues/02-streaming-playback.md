# 02 — Streaming playback

**What to build:** The viewer plays the open case as a stream. The binding gains a stream session that can push a chunk, set the whole text so far, and return the current result. Parsing runs on a dedicated thread with a 1 MB stack, and the UI never calls the core itself.

The playback controls are:
- chunk size and speed;
- play, pause, and step forward or back one chunk;
- a slider to jump to any chunk.

Elements still arriving carry a subtle "still arriving" marker. Each chunk's time is shown, split into parsing, crossing to Swift, decoding and redrawing.

Playback is driven by the sources-of-chunks interface. A replay of a corpus case or file, at a chosen chunk size, is its first implementation.

**Blocked by:** 01 — One corpus case, drawn as boxes

**Status:** ready-for-agent

- [ ] For every corpus case, the binding's stream result equals the core's at every step, under several chunkings including one character at a time. This test runs in `scripts/check.sh`.
- [ ] Play, pause, step forward, step back, the slider, chunk size and speed all work.
- [ ] Stepping back shows exactly the result the earlier step had.
- [ ] Parsing happens only on the dedicated thread, which has a 1 MB stack. In debug builds, any call to the core from the main thread fails loudly.
- [ ] Partial elements are visibly marked.
- [ ] Each chunk's time is shown, split into parse, crossing, decoding and redrawing.
- [ ] The UI stays responsive while the largest corpus program plays one character per chunk: the slider and pause keep working.
- [ ] A Swift test checks that the replay source yields a known program's chunks in order, with the right boundaries, for several chunk sizes.
