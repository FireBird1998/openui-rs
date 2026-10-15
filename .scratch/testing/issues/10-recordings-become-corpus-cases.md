# 10 — Recordings become corpus cases

**What to build:** A command that imports the viewer's recordings into the corpus. The recording format is documented on `Recording` in the viewer.
- **A fresh recording** becomes a parse case: the program it received, in the recording's library. It gets a **recorded chunking** taken from its deltas' sizes in code points. The reference's runner learns to accept a chunking with explicit sizes and computes its golden alongside the corpus's other chunkings. The offline suite then checks every step of it like any other.
- **A follow-up recording** becomes a merge case, with the program it patched as the existing program and its text as the patch. It also becomes a parse case of the merged program.

The request and model are kept as case metadata. Recordings never hold a key.

Prove it with a hand-made recording in the tests, and with one real recording the maintainer makes in the viewer, which needs their API key.

**Blocked by:** None — can start immediately (the real recording needs the maintainer's key).

**Status:** ready-for-agent

- [ ] One command imports recordings from a folder into the corpus and regenerates their goldens.
- [ ] The reference's runner computes goldens for a recorded chunking with explicit sizes.
- [ ] The offline suite checks a recorded chunking at every step.
- [ ] A follow-up recording becomes a merge case and a parse case of its merged program.
- [ ] One real recording, made by the maintainer, is imported and conforms.
- [ ] CONFORMANCE.md's counts include recorded cases, and its report test checks them.
- [ ] `scripts/check.sh` passes.
