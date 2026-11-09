# 10 — Recordings become corpus cases

**What to build:** A command that imports the viewer's recordings into the corpus. The recording format is documented on `Recording` in the viewer.
- **A fresh recording** becomes a parse case: the program it received, in the recording's library. It gets a **recorded chunking** taken from its deltas' sizes in code points. The reference's runner learns to accept a chunking with explicit sizes and computes its golden alongside the corpus's other chunkings. The offline suite then checks every step of it like any other.
- **A follow-up recording** becomes a merge case, with the program it patched as the existing program and its text as the patch. It also becomes a parse case of the merged program.

The request and model are kept as case metadata. Recordings never hold a key.

Prove it with a hand-made recording in the tests, and with one real recording the maintainer makes in the viewer, which needs their API key.

**Blocked by:** None — can start immediately (the real recording needs the maintainer's key).

**Status:** waiting for one real recording by the maintainer (needs their API key); everything else done.

- [x] One command imports recordings from a folder into the corpus and regenerates their goldens.
- [x] The reference's runner computes goldens for a recorded chunking with explicit sizes.
- [x] The offline suite checks a recorded chunking at every step.
- [x] A follow-up recording becomes a merge case and a parse case of its merged program.
- [ ] One real recording, made by the maintainer, is imported and conforms.
- [x] CONFORMANCE.md's counts include recorded cases, and its report test checks them.
- [x] `scripts/check.sh` passes.

## Comments

**2026-10-16:** upstream released lang-core 1.0, and 27 of 47 system prompts change with it. If the pin moves (ticket 14), move it before recording, so the recordings test the new language.

## Results

Checked 2026-11-09.
- **Import:** `scripts/import-recordings.sh` (3019307) imports recordings into `conformance/cases/recorded/`. It turns a follow-up into a merge case and a parse case of the merged program. `tests/recordings.rs` checks a recorded chunking at every step.
- **Proof:** two hand-made recordings prove the path. CONFORMANCE.md counts them (2 recorded parse cases, 1 recorded merge), and the report test checks the counts.
- **Still open:** one real recording made in the viewer by the maintainer with their key. The pin has since moved to lang-core 1.1, so record against 1.1.
