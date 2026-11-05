# 02 — Tokens and statements: offsets, and comments in the lexer

**What to build:** The tokens and statements stages match 1.1:
- identifiers, type names, `$state`s, newlines and EOF carry their source offset `start`;
- statements carry `start` and `end`;
- the lexer skips `//` and `#` line comments to the end of the line.

Offsets count what JavaScript's string indices count: UTF-16 code units of the text the lexer gets.

**Blocked by:** 01 — Vendor lang-core 1.1.0

**Status:** done (2026-11-06)

- [x] No case is pending in `tokens.txt` or `statements.txt`.
- [x] Cases cover a comment in the middle of a statement, `#` and `//` at the start of a line and after code, and offsets after non-ASCII text.
- [x] `scripts/check.sh` passes.

## Comments

**2026-11-06, done.**
- **Tokens:** `tokenize` returns `Tokens`, which reads as the token slice and keeps each token's start as a byte offset and a UTF-16 offset. The UTF-16 offset is counted from the bytes as they're lexed. `tokens_to_json` writes `start` on the five kinds the reference gives one.
- **Statements:** `RawStatement` keeps its tokens' starts, and its own `start` and `end`. Merging (ticket 04) slices with the byte offsets.
- **Comments:** the lexer skips `//` and `#` to the end of the line, checked before `/`, as the reference does.
- **New cases:** `hand/112-comments-inside-statements` and `hand/113-offsets-after-non-ascii`. The second has emoji, which are two UTF-16 units each, and letters that cut identifiers.
- **Result:** nothing pending in `tokens.txt` or `statements.txt` (605 and 599 before).
