# 05 — Fences, comments and prose around a program

**What to build:** The preprocessing the reference parser does before parsing: stripping markdown fences, comments, and prose before or after the program. Stage-level conformance against the reference package's exported `stripFences`, plus new corpus cases for the ways LLMs actually wrap output.

**Blocked by:** 02 — Real-world corpus.

**Status:** done (2026-10-06)

- [x] Fence stripping matches the reference parser for every corpus program and prefix.
- [x] New cases cover: fenced and unfenced programs, a fence that opens mid-stream, prose with apostrophes before the program, comment-like text inside strings, and trailing prose.
- [x] Stripping arbitrary text never panics.

## Comments

**From ticket 01:** preprocess is currently JS `trim` only (`js::trim` reproduces JS's whitespace set: it includes U+FEFF and excludes U+0085). `stripComments` carries its in-string state across lines, so a quote opened on one line affects comment detection on the next. Port that faithfully.

**2026-10-06, done.**
- New `preprocess` module: `strip_fences` and `preprocess` are public (trim, fences, comments, trim); `strip_comments` is private, as in the reference. `js::trim_end` added. Both parse and the stream parser use it, replacing the trim-only stand-in.
- The reference keeps `stripComments` and `preprocess` private, so `reference/run.ts` registers a Bun loader plugin that appends `export { stripComments, preprocess }` to `parser.ts` as it loads. The vendored file stays byte-identical to upstream.
- Stage golden: `<case>.preprocess.json` fingerprints `{stripFences, preprocess}` per prefix (`bun run.ts show-preprocess <case> <length>`). All 277 cases match on every prefix.
- New hand cases 72–77: a fence opening mid-stream after unfenced statements, prose with apostrophes, comment-like text in strings (URLs, `#`, a multi-line string with `//`), trailing prose with inline code and a ```, backticks inside strings plus a second fence, and CRLF with tabs.
- Deliberate breaks, each caught: resetting quote state per line (hand 74, test/parser-26/27), finding fences inside strings (hand 76, test/parser-24/25), and skipping `trimEnd` before comments (hand 28, 70, 74).
- `tests/robustness.rs`: strip_fences and preprocess on any text and on fence/comment-heavy text (2 × 2000 cases).
- Whole-result conformance went from 29/271 to 37/277. Seven pending cases now conform (fenced and commented programs from the docs, examples and reference tests), and so does the new hand/73. The other new cases wait on ticket 06.
- **First quirk found** (corrected after testing variants against the reference): an odd number of apostrophes in unfenced prose before the program makes `autoClose` and the lexer read the rest as one `'...'` string. You get 0 statements, `root: null`, and nothing renders. The first write-up blamed `//` comments; that was wrong, they're irrelevant. With trailing prose, everything renders but stays `incomplete`/`partial`. Pinned by hand/78 and hand/79, and written up in `conformance/QUIRKS.md` with a variant table and a draft upstream report (not filed).
