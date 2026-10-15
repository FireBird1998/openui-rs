# 03 — Coverage map

**What to build:** A line-coverage report for the core over the offline suite, the same tests the check runs, from one command. Use it to leave no unrun line unexplained. Each line no test runs is either:
- covered by a new case;
- deleted as dead code;
- or listed with the reason it can't be reached, in a committed file.

There's no percentage target. The bar is nothing left unexplained.

**Blocked by:** None — can start immediately.

**Status:** done (2026-10-15)

- [x] The maintainer approves installing cargo-llvm-cov, and the Rust component it needs, before either is installed.
- [x] One command produces the report: a summary in the terminal and per-file detail outside git.
- [x] Every unrun line in the core is covered, deleted, or listed with its reason in a committed file.
- [x] New conformance cases come with the reference's goldens.
- [x] TESTING.md has a section on coverage: running it and reading it.
- [x] `scripts/check.sh` passes.

## Comments

**2026-10-15, done.** cargo-llvm-cov 0.9.1 and `llvm-tools-preview` installed with the maintainer's approval. `scripts/coverage.sh` runs the offline suite with live diffs off, prints the summary, writes HTML to `target/llvm-cov/html/`, and fails if an unrun line isn't listed in `crates/openui-lang/UNREACHED.md`. About a minute.

- **Before:** 96.95% of lines, 152 not run.
- **After:** 99.12%. The 30 source lines still not run are each listed with a reason: guards that mirror the reference, invariant checks, a compile-time check. The summary says 44 because it counts generic code once per compiled copy.
- **Dead code deleted:** `Ast::is_runtime_expr`, unused anywhere.
- **New cases, all matching the reference:**
  - nine parse cases (`hand/87` to `hand/95`) on a new `edge` schema: required props with defaults, enums of booleans, props named like `Object.prototype` methods, `properties` as an array, element-shaped objects, queries and objects as values, number literal keys, a query member in a prop, odd `required` and `properties`;
  - three prompt specs: a spec that's an array, a cloud library that isn't an object, tools with odd input schemas;
  - one serialize case: a numeric statement id as the last argument.
- **New tests:** `api` (the public JSON reader and writer, `JsObject`, walking an expression), two CLI tests (stdin, the default chunk size), and the binding's error messages.

**Findings, for review together:**
1. **New divergence, documented in CONFORMANCE.md.** A component with `properties` whose `required` is a number: the reference throws a `TypeError` while building the library (`required.includes is not a function`); the core treats `required` as empty. Throwing would turn a bad schema into a failed parse in the core too, so this stays as is unless we decide otherwise.
2. **Prompt errors from the JavaScript engine.** For a spec that's a JSON array, the reference crashes with an engine `TypeError` rather than its own message. Prompt goldens now record `errorType` for those, and the test only asks the core to fail too. Messages the reference writes itself are still compared exactly.
3. **`scripts/regen-golden.sh` deletes the goldens before writing them.** When the reference crashed on a case, it left the goldens deleted; `git checkout -- conformance/golden` restored them. Worth a fix (write to a temporary folder, then swap) as a small change or in ticket 02.
4. **One branch in `stream.rs` is unproven either way:** nothing pending after completed statements. No input tried reaches it while streaming. It's listed, not deleted, until a hunt shows otherwise; ticket 09's invariants are a good place to try.
