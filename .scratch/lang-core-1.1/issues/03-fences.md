# 03 — Fences: the line-based extraction

**What to build:** Preprocessing extracts the program from fences as 1.1 does:
- line by line, with an inline ```` ``` ```` pair ignored;
- code allowed on the opener line;
- `openui-js` blocks skipped;
- an unfenced program used when no block starts one;
- a fence line inside a multi-line string kept in the string (#1554).

Quirk 1 (an apostrophe in unfenced prose) is re-checked against it.

**Blocked by:** 01 — Vendor lang-core 1.1.0

**Status:** ready-for-agent

- [ ] No case is pending in `preprocess.txt`, and the parse cases pending only for fences match.
- [ ] New cases cover each bullet of the spec's section 2.1, while streaming as well.
- [ ] Quirk 1 is re-checked: still a quirk, or fixed upstream and recorded as such.
- [ ] `scripts/check.sh` passes.
