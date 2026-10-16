# 02 — Vendor lang-core 1.0.1, its tests and the spec's fixtures

**What to build:** The reference is lang-core 1.0.1 (`eb20708`), every golden comes from it, and the corpus holds what 1.0 adds to test against. The core's remaining differences from 1.0 are all pending, so the check passes and the gap is listed case by case.

- **The reference:** vendored at the new commit, with `zod`, which 1.0 imports at run time, in its packages. A local git tag marks the last commit that conforms to 0.3.0.
- **Stream end:** every stream in the goldens ends with a stream-end step (upstream's `getResult(true)`), and so does the fuzzer's. The core gets `StreamParser::finish()`, which for now gives the same result as `result()`. The `$name` rule comes in ticket 07.
- **Upstream's new tests:** imported at the new commit. The test capture rebuilds a library's `properties`, `functions` and `actions` from what the parser was given.
- **The spec's fixtures:** the 54 cases of `spec/fixtures/1.0/` become corpus cases, keeping where each came from:
  - each program is a parse case;
  - a chunked case keeps its chunks, as their own case;
  - each edit (program plus patch) is a merge case;
  - the fixture library is both a schema and a prompt spec.
- **Goldens:** regenerated. Whatever the core doesn't match yet is pending.

**Blocked by:** 01 — Pending lists for every golden test.

**Status:** done (2026-10-16)

- [x] The reference runs lang-core 1.0.1, and `reference/UPSTREAM` names it.
- [x] A local tag marks the last 0.3.0-conformant commit.
- [x] Goldens end every stream with a stream-end step, and the core has `finish()`.
- [x] Upstream's 1.0 tests and the 54 fixtures are corpus cases with their sources recorded.
- [x] Every golden is regenerated, and the pending lists hold exactly what differs.
- [x] CONFORMANCE.md says the move is in progress, with the pending counts.
- [x] `scripts/check.sh` passes.

## Comments

**2026-10-16, done.** The reference is lang-core 1.0.1 (`eb20708`), with `zod` 4.6.7 in its packages, and the local tag `lang-core-0.3.0` marks the last 0.3.0-conformant commit (`e258597`).

- **Stream end:** every stream's last golden step is `getResult(true)`: one more step for each push chunking, and an `e` op after the `r` that ends each resync sequence. The core has `StreamParser::finish()`, the same as `result()` until ticket 07. The fuzzer and `serve.ts` end their streams too.
- **Recorded chunks:** a case may carry `recorded` chunk sizes, giving it a `recorded` chunking whose sizes its golden records as well. The spec's 4 streaming fixtures use it, and the viewer replays them like the random chunkings.
- **The corpus at 1.0.1:**

  | | Before | Now |
  |---|---:|---:|
  | Parse cases | 311 | 524 |
  | from upstream's tests | 74 | 196 |
  | from the spec's fixtures | – | 58 |
  | from the docs | 72 | 105 |
  | Merges | 1,417 | 2,521 |
  | Trees back to source | 342 | 501 |
  | Enriched errors | 191 | 329 |
  | Prompts | 47 | 49 |

  Upstream's tests come from all three test folders now: the parser's, the runtime's (library functions and actions) and the package's. The query manager's tests are skipped; they parse nothing and use vitest's fake timers, which bun test lacks. The capture rebuilds `properties`, `functions` and `actions`. The generated merges were regenerated over the new corpus.
- **The spec's fixtures:** 54 programs plus 4 streamed ones become parse cases, 5 edits become merge cases, and the fixture library becomes the schema `spec-1.0` and two prompt specs.
- **Pending:** 520 of 524 parse cases, 63 statements and expression trees, 41 trees back to source, 229 enriched errors, 29 of 49 prompts. Tokens, preprocessing and merging: none, as their code didn't change.
- **Tests:** the CLI tests now compare the CLI with the library's own output, not with goldens. The enriched-errors test also checks golden entries the core produces no errors for. The one-shot test doesn't assert while `UPDATE_PENDING` rewrites the list. The viewer's golden tests skip pending cases, and its goldens keep the stream end apart from the pushes.
- **Tools:** `scripts/update-reference.sh` warns when the vendored lang-core imports a package `reference/package.json` lacks (as `zod` was). The upstream check reports the pin current.
- **Checked:**
  - `scripts/check.sh` and `scripts/check-viewer.sh` pass;
  - coverage is unchanged at 99.12% of lines, all explained;
  - the fuzzer runs against 1.0.1: 309 of 400 programs mismatch, as expected mid-move, all merges match, and 4 of 200 serializations differ.
