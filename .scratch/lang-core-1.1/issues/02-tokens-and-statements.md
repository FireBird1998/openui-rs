# 02 — Tokens and statements: offsets, and comments in the lexer

**What to build:** The tokens and statements stages match 1.1:
- identifiers, type names, `$state`s, newlines and EOF carry their source offset `start`;
- statements carry `start` and `end`;
- the lexer skips `//` and `#` line comments to the end of the line.

Offsets count what JavaScript's string indices count: UTF-16 code units of the text the lexer gets.

**Blocked by:** 01 — Vendor lang-core 1.1.0

**Status:** ready-for-agent

- [ ] No case is pending in `tokens.txt` or `statements.txt`.
- [ ] Cases cover a comment in the middle of a statement, `#` and `//` at the start of a line and after code, and offsets after non-ASCII text.
- [ ] `scripts/check.sh` passes.
