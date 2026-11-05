# 03 — Fences: the line-based extraction

**What to build:** Preprocessing extracts the program from fences as 1.1 does:
- line by line, with an inline ```` ``` ```` pair ignored;
- code allowed on the opener line;
- `openui-js` blocks skipped;
- an unfenced program used when no block starts one;
- a fence line inside a multi-line string kept in the string (#1554).

Quirk 1 (an apostrophe in unfenced prose) is re-checked against it.

**Blocked by:** 01 — Vendor lang-core 1.1.0

**Status:** done (2026-11-06)

- [x] No case is pending in `preprocess.txt`, and the parse cases pending only for fences match.
- [x] New cases cover each bullet of the spec's section 2.1, while streaming as well.
- [x] Quirk 1 is re-checked: still a quirk, or fixed upstream and recorded as such.
- [x] `scripts/check.sh` passes.

## Comments

**2026-11-06, done.**
- **One-shot:** `strip_fences` is a direct port of 1.1's line-by-line rules. That covers fence lines and their regex, `.*?` refusing line terminators other than a final `\r`, code on the opener line, `openui-js`, inline pairs, and choosing blocks or outside text. Nothing is pending in `preprocess.txt` (8 before), and 7 parse cases that were pending only for fences match.
- **Streaming:** `Preprocessor` is rewritten for the new rules:
  - every line before the last one with text in it goes through them once, into one of two comment stages, the text inside blocks and outside them;
  - the last line, trimmed as the end of the text is, is applied tentatively on each push;
  - `cleaned` comes from the stage the rules choose;
  - a block's "starts a program" is kept only until more lines can't change it.

  Its unit tests gained 1.1's inputs: script blocks, inline pairs, opener code, the tag as the name, stray closing fences, `js` helpers, fences inside multi-line strings, comment lines, `\s` across lines, and U+2028 after a fence. `streaming_cost` gained a fenced program with prose around it: 1.18× for the 4× program, against 1.22× bare.
- **New cases:** `hand/114` to `hand/124`, one per rule in the spec's section 2.1. All match the reference, streaming included.
- **Quirk 1:** still so in 1.1. The fence rules don't touch unfenced prose, and the apostrophe still opens a string in `autoClose`, the lexer and comment stripping.
