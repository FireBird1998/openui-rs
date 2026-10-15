# 07 — Recording and replay

**What to build:** The maintainer can save a live stream as a recording, in a folder chosen in settings. A recording holds:
- the request;
- the model;
- the component library's name;
- when it was made;
- every text delta exactly as it arrived, with its arrival time.

Recordings open like any other program and replay through a recording source of chunks, either at their recorded timing or as fast as possible. The format is documented, so the testing phase can turn recordings into corpus cases with recorded chunkings.

**Blocked by:** 06 — Live streaming from Claude

**Status:** ready-for-agent

- [ ] Saving a live stream writes a recording with every field listed above.
- [ ] The recording format is documented for the testing phase.
- [ ] Replaying a recording yields exactly the recorded chunks, in order. A Swift test checks this with a hand-made recording.
- [ ] Replay runs at the recorded timing, or as fast as possible.
- [ ] Recordings never contain the API key or request headers. This is tested.
- [ ] The maintainer has recorded and replayed one real stream.
