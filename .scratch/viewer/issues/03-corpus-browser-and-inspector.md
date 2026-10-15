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

**Status:** ready-for-agent

- [ ] Every corpus case is listed, and each opens with its own component library and root name.
- [ ] Program files open with a chosen library, and pasted text works.
- [ ] The inspector shows the tree, the incomplete, unresolved and orphaned lists, the errors and the limit flags for the current step.
- [ ] Selection works both ways between the drawing and the tree.
- [ ] The text received so far is highlighted in the program text.
- [ ] Golden match shows a pass or a fail for the current step when a golden exists, and "no golden for this chunking" otherwise. A test with a deliberately wrong fingerprint shows a fail.
- [ ] The viewer reads the conformance folder of the repository it was built from, and settings can point it elsewhere.
- [ ] A Swift test decodes every corpus case's golden one-shot result into the Swift model.
