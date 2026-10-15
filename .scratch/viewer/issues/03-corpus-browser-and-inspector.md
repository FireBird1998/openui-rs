# 03 — Corpus browser and inspector

**What to build:** The maintainer can open three things:
- any corpus case, from a browsable and searchable list grouped by where the cases came from;
- a program file, with a component library they choose;
- pasted text.

Beside the drawing, an inspector shows the parse result at the current step:
- the element tree;
- what's incomplete, unresolved and orphaned;
- validation errors, with their component, path and statement;
- the depth-limit and size-limit flags.

Selecting an element in the drawing selects it in the tree, and the other way round. The program text is shown with the part received so far highlighted. For corpus cases played with a chunking that has a golden, the inspector says whether the current step matches the reference's golden.

**Blocked by:** 02 — Streaming playback

**Status:** done (2026-10-15)

- [x] Every corpus case is listed, and each opens with its own component library and root name.
- [x] Program files open with a chosen library, and pasted text works.
- [x] The inspector shows the tree, the incomplete, unresolved and orphaned lists, the errors and the limit flags for the current step.
- [x] Selection works both ways between the drawing and the tree.
- [x] The text received so far is highlighted in the program text.
- [x] Golden match shows a pass or a fail for the current step when a golden exists, and "no golden for this chunking" otherwise. A test with a deliberately wrong fingerprint shows a fail.
- [x] The viewer reads the conformance folder of the repository it was built from, and settings can point it elsewhere.
- [x] A Swift test decodes every corpus case's golden one-shot result into the Swift model.

## Comments

**2026-10-15, done.** `scripts/check.sh` and `scripts/check-viewer.sh` pass, with 21 Swift tests. The new tests:
- The reference's one-shot result for all 300 corpus cases decodes into the viewer's model.
- A deliberately wrong fingerprint shows as a fail; step 0 and chunkings without goldens say so.
- Three cases (`hand/04`, `hand/71`, `bench/dashboard`) are played step by step through the viewer at 3 and 16 code points per chunk and under the corpus's recorded random chunkings. Every step matches the reference's golden.
- The element tree's paths match the drawing's.
- Search and grouping work.

**How it works:**
- **Golden match.** The inspector computes the shown step's FNV-1a fingerprint, the same one `reference/run.ts` writes, and compares it with `golden/<case>.steps.json` for the current chunking. For corpus cases, the chunk menu also offers the case's seeded random chunkings, using the sizes their golden records. The resync chunkings mix in `set` calls, which playback doesn't model, so they aren't offered.
- **Selection.** Every element gets a path from the root, such as `/children/2/header`. The drawing and the tree both use it as the element's identity, so a selection survives new chunks. Clicking a box selects its tree row and scrolls the tree to it; picking a tree row outlines the box and scrolls the drawing to it. Only the small outline views watch the selection, so selecting doesn't redraw the boxes.
- **Program text.** An AppKit text view under the drawing. Each step only recolors it: the received text in full color, the latest chunk highlighted, the rest faded. Playback now tracks each step's offset in UTF-16 units for this.
- **Opening programs.** The toolbar's open button (doc with a plus) takes pasted text or a file, with a corpus library or a JSON Schema file, and an optional root name. Such programs have no golden.
- **Settings** (⌘,): the conformance folder, stored in the app's defaults under its bundle id. Empty means the repository the viewer was built from.

Checked in the running app: the search narrows the list, the "Matches the reference" row stays green while `bench/dashboard` plays at 30 chunks/s, and the tree, errors and text highlight follow the steps. While I was checking, the window was resized, a case was selected and Play was pressed, none of it by me. It looked like the maintainer was using the viewer, so I stopped clicking in it. Two-way selection by mouse wasn't tried by hand.
