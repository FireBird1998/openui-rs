# 04 — Merging: splitting like the parser

**What to build:** `merge_statements` matches 1.1:
- it splits with the parser's own statement splitting;
- each statement's text is its source span, trimmed at the end;
- a statement defined twice in the existing program keeps its first position.

The merge quirks are re-checked: 6, 7, 8, 15, 16 and 20.

**Blocked by:** 02 — Tokens and statements

**Status:** done (2026-11-06)

- [x] No case is pending in `merge.txt`.
- [x] Each of quirks 6, 7, 8, 15, 16 and 20 is either still a quirk, with its write-up updated, or recorded in QUIRKS.md as fixed in 1.1.
- [x] `scripts/check.sh` passes.

## Comments

**2026-11-06, done.**
- **Merging:** `merge_statements` splits with the parser's `split(tokenize(...))` and slices each statement's source span with ticket 02's byte offsets, trimmed at the end. An id defined twice keeps its first place. Nothing is pending in `merge.txt` (63 before): 3,118 merges match.
- **The merge quirks on 1.1.0:**

  | Quirk | Now |
  |---|---|
  | 6, a ternary's continuation lines dropped | fixed |
  | 7, an apostrophe in prose before the program | still so |
  | 8, an id defined twice comes out twice | fixed |
  | 15, appending after an unfinished statement | still so |
  | 16, merging into an empty program | still so |
  | 20, comments read as code | fixed |

  QUIRKS.md keeps the fixed ones, under their numbers, in "Fixed upstream". CONFORMANCE.md strikes them through, and its count of quirks is now 17.
- **Fuzzer:** the merge rules no longer excuse breaks by quirks 8 and 20. Quirk 7's exception now says it's the prose around the existing program's fences.
