# 04 — Merging: splitting like the parser

**What to build:** `merge_statements` matches 1.1:
- it splits with the parser's own statement splitting;
- each statement's text is its source span, trimmed at the end;
- a statement defined twice in the existing program keeps its first position.

The merge quirks are re-checked: 6, 7, 8, 15, 16 and 20.

**Blocked by:** 02 — Tokens and statements

**Status:** ready-for-agent

- [ ] No case is pending in `merge.txt`.
- [ ] Each of quirks 6, 7, 8, 15, 16 and 20 is either still a quirk, with its write-up updated, or recorded in QUIRKS.md as fixed in 1.1.
- [ ] `scripts/check.sh` passes.
