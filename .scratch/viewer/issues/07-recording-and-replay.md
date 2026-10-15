# 07 — Recording and replay

**What to build:** The maintainer can save a live stream as a recording, in a folder chosen in settings. A recording holds:
- the request;
- the model;
- the component library's name;
- when it was made;
- every text delta exactly as it arrived, with its arrival time.

Recordings open like any other program and replay through a recording source of chunks, either at their recorded timing or as fast as possible. The format is documented, so the testing phase can turn recordings into corpus cases with recorded chunkings.

**Blocked by:** 06 — Live streaming from Claude

**Status:** done (2026-10-15), except the maintainer's check with a real stream

- [x] Saving a live stream writes a recording with every field listed above.
- [x] The recording format is documented for the testing phase.
- [x] Replaying a recording yields exactly the recorded chunks, in order. A Swift test checks this with a hand-made recording.
- [x] Replay runs at the recorded timing, or as fast as possible.
- [x] Recordings never contain the API key or request headers. This is tested.
- [ ] The maintainer has recorded and replayed one real stream.

## Comments

**2026-10-15, built and tested; one check left for the maintainer.** `scripts/check-viewer.sh` passes with 49 Swift tests. The new ones:
- **The file:** a hand-made recording survives a round trip; other JSON is rejected.
- **The chunks:** its recorded chunking cuts the text exactly where the deltas arrived, including where a delta splits a letter from its combining accent. Replaying it all at once gives exactly the recorded chunks.
- **The timing:** replaying it as recorded gives them in order, taking at least the recorded time.
- **Playback** replays a recording as recorded.
- **Saving:** saved recordings list newest first, and junk files are skipped.
- **No secrets:** a recording made from a stubbed live stream holds what arrived and no key or header names. This was checked against the request's real header values.

**The format.** It's documented on `Recording` in `viewer/Sources/ViewerKit/Recording.swift`, and the testing spec points there. One `.recording.json` file per stream holds:
- `format` and `version`;
- `createdAt`, `request`, `model` (asked) and `servedBy` (answered), `effort` and `library`;
- `stopReason`, which is null when the stream was stopped or failed;
- `inputTokens` and `outputTokens`;
- `chunks`: each text delta exactly as received, with `t`, seconds from sending the request.

**How it works:**
- **Timing.** The live source stamps each delta when it arrives and records it in the live status.
- **Saving.** Once a live request is over and something arrived, the live bar offers "Save Recording". After saving it becomes "Saved", which shows the file in Finder.
- **Where.** Files go to Application Support (`dev.openui-rs.viewer/Recordings`) by default, which avoids macOS's prompt for access to Documents. Settings can choose another folder, or show it in Finder.
- **Opening.** Recordings appear in a "Recordings" section at the top of the sidebar, newest first, and search covers them. A recording opens all at once, cut where its deltas arrived: the chunk menu's "as received" entry, which comes from the chunk sizes. A bar above playback shows the model, date, chunk count, duration and how it ended, with "Replay as Recorded", which follows the original timing, and "Show in Finder".

**Left:** the maintainer records and replays one real stream, which needs the API key from ticket 06.

