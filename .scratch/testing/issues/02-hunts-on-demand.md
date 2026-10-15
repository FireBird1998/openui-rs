# 02 — Hunts on demand

**What to build:** A hunt: one command that searches for findings for a set time, then leaves a report and shrunk cases for review.

`scripts/hunt.sh --minutes N` runs, within its budget:
- differential fuzzing with a fresh seed, for parsing and for edits (merge and conversion back to source);
- an upstream check: are there lang-core commits newer than the pin? When there are, it vendors the new reference into a scratch copy, computes goldens for our corpus there, and reports which cases' outputs changed. Moving the pin stays a separate ticket.

Everything goes into a dated report in a findings folder git ignores. A hunt never commits and never changes tracked files. Today the fuzzer saves shrunk cases into the corpus, so a hunt needs it to save them into the findings folder instead. The report sorts each finding into a mismatch, a crash, a blow-up or a new quirk as far as the hunt can tell, with its seed and how to reproduce it. Later tickets add their checks to the hunt.

**Blocked by:** None — can start immediately.

**Status:** done (2026-10-15)

- [x] One command with a time budget runs differential fuzzing for parsing and edits with a fresh seed, and stops within the budget.
- [x] Shrunk findings and a dated report land in a folder git ignores. A test or check confirms that no tracked file changes during a hunt.
- [x] Each finding in the report has its kind as far as the hunt can tell, its seed, and how to reproduce it.
- [x] The upstream check reports new lang-core commits since the pin and, when there are any, which corpus cases' outputs they change. Without network or Bun, it says so and the hunt goes on.
- [x] TESTING.md has a section on hunts: running one, reading its report, and turning a finding into a case.
- [x] `scripts/check.sh` passes.

## Comments

**2026-10-15, done.** `scripts/hunt.sh --minutes N [--seed S] [--no-upstream]` runs differential fuzzing (parsing for the first half, then edits) with the upstream check alongside, and writes `findings/<date>-seed<seed>/report.md` plus the shrunk cases in the corpus's formats. Parts:
- **The fuzzer** learned `--seconds` (no batch starts in its last 5 s; shrinking stops at the end and keeps what it has), `--from`, `--findings DIR` (cases, `findings.jsonl`, a progress file naming the batch running, and per-check summaries go there instead of the corpus) and `--slow-ms` (default 1000: a slower core answer is a blow-up). `reference/serve.ts` reports its own time per answer.
- **`scripts/upstream-check.sh <folder>`** keeps upstream's history without file contents in `target/upstream.git` (3.3 MB), lists lang-core commits since the pin, and with new ones vendors the newest into the folder, installs what it imports there, computes goldens and lists the changes. Falls back to the cached history without network, and to commits only without Bun.
- **`scripts/regen-golden.sh`** now computes into a scratch copy and swaps the goldens in only when every script succeeds (ticket 03's third finding), and takes `--lang-core`, `--node-modules` and `--into` for the upstream check.
- **`scripts/hunt.py`** writes the report and compares golden folders.
- **The tracked-file check:** a hunt hashes every tracked and untracked-but-not-ignored file before and after, and exits 1 naming any that changed.

**Tested:** a 3-minute hunt ran 2 min 42 s: 80,600 programs, about 1.34 million edits, and the upstream check, with git status unchanged afterwards. Failures tried on purpose: an aborted fuzzer is reported as a crash, with its last batch as the replay; a hung one is stopped at its time and reported as a blow-up; a file added mid-hunt fails the hunt; without Bun only the upstream check runs; without network it reads the cache, and without a cache it says so. No processes were left behind.

**Findings, for review together:**
1. **Mismatch: a root that's an element-shaped object literal.** `x={type:"element",typeName:"A",partial:false,props:{}}`: the reference's root is the object, tagged with `statementId`; the core's is null. The core types `ParseResult.root` as an `Element`, and the reference keeps the object's own keys and their order, so this is a design question about the root's type, or a documented divergence (no model writes this).
2. **Mismatch: `hasDynamicProps` over an element-shaped object holding an expression.** `t=Probe({type"element"typeName""partial false props][` streamed: the reference sets `hasDynamicProps: true` on Probe, and the core sets false.
3. **Mismatch: converting back to source, props null or missing.** For a component whose schema has no usable `properties` (`Weird` in the edge schema, `Empty` in the odd one), the reference writes `root = Weird()` and the core refuses: "the root element has no props".
4. **Upstream moved: OpenUI 1.0.** 11 lang-core commits since the pin, newest `eb20708` (lang-core 1.0.1). With it, outputs change for 305 of 309 parse cases, 25 of 335 conversions back to source, 77 of 191 error hints and 27 of 47 system prompts. Two causes seen so far: streaming results whose root isn't a component yet now carry a `no-root` error, and built-in action calls carry `mappedProps`. It also imports `zod` (a peer dependency) at run time. Its runtime telemetry is opt-in and respects our opt-out variables. Moving the pin is a separate decision.

Findings 1 to 3 all come from the `edge` and `odd` schemas, which ticket 03 added to the fuzzer's choices. The hunts' folders are in `findings/` for review.

**2026-10-16, the findings reviewed:**
- **1:** already a documented divergence. The fuzzer knew only the `__proto__` one, and now recognizes this one too (`0610c25`).
- **2 and 3:** fixed in the core, each with its cases first (`2dd2623`). A 2-minute hunt afterwards found nothing in 73,600 programs and 1.4 million edits.
- **4:** became ticket 14, which waits for the maintainer's decision to move.
