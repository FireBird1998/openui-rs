# Quirks

Behaviour of the reference parser that looks wrong but that the core reproduces anyway, because the contract is to match the reference byte for byte. Fixing a quirk happens upstream first; the core follows when the pinned commit moves.

Each entry has a minimal program, what the reference does, the corpus case that pins it, and a draft upstream report. Nothing here has been filed yet.

## 1. An apostrophe in leading prose stops comments from being stripped

- **Found:** 2026-10-06, while porting `stripComments` (ticket 05). Reference commit `bba1d251`.
- **Pinned by:** `hand/78-apostrophe-hides-comment`, and `hand/73-prose-with-apostrophes` for the prose part.

```
Here's the UI:
root = Root([a]) // main layout
a = Text("x")
```

**Reference behaviour:** `root` is `null`, and the UI renders nothing. With "Here is the UI:" instead, `root` is the `Root` element as expected.

**Why:** `stripComments` treats `'` as a string delimiter and carries its in-string state from one line to the next. The apostrophe in "Here's" opens a string that never closes, so every later `//` or `#` comment is kept. `// main layout` then tokenizes as `/ / main`, and `root`'s expression becomes `BinOp(Root(...), "/", null)`, which isn't an element.

**Draft upstream report:**

> **Leading prose with an apostrophe disables comment stripping, so the whole UI can disappear.** When a model response starts with prose containing an apostrophe (such as "Here's the UI:") and any later statement has a trailing `//` or `#` comment, `parse()` returns `root: null`. `stripComments` in `packages/lang-core/src/parser/parser.ts` treats `'` as a string delimiter and carries that state across lines, so the apostrophe opens a string that hides every later comment. `stripFences` was already made apostrophe-safe (see the "does not treat apostrophes as string delimiters when stripping fences" test); `stripComments` needs the same treatment, or at least to reset single-quote state at each newline. Repro: `parse("Here's the UI:\nroot = Root([a]) // main layout\na = Text(\"x\")", lib)`.
