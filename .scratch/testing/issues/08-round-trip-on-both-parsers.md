# 08 — Round trip on both parsers

**What to build:** The first invariant, run on the core and the reference alike.

**The engine interface.** It has two implementations:
- **the core**, in-process;
- **the reference**, through the fuzzer's long-lived reference process, which learns to return full results instead of only fingerprints.

The interface covers four operations:
- a full one-shot result;
- a stream over given chunks;
- merging;
- conversion back to source.

**The rule** is written once against that interface: parsing, converting back to source and parsing again gives the same tree, ignoring the partial and dynamic-prop flags the way upstream's own round-trip test does.

**Running it.** It's checked on a million generated programs. Every failure is sorted:
- **Broken by both parsers:** a candidate quirk. The maintainer judges whether the reference is wrong, which makes it a quirk with a draft upstream report and an exception to the rule, or the rule is wrong, which narrows the rule.
- **Broken by the core only:** a mismatch to fix.

Quirk 9, the dropped parentheses, is a known exception. Hunts then run the rule.

**Blocked by:** 02 — Hunts on demand

**Status:** done (2026-10-16); the maintainer's judgments recorded 2026-11-09 (below)

- [x] The engine interface exists, with both implementations; the reference process returns full results.
- [x] The round-trip rule is written once and runs on both.
- [x] It has run on a million generated programs, and every failure is sorted.
- [x] Any new quirk is in QUIRKS.md with a draft upstream report, which isn't filed.
- [x] The rule's exceptions are listed beside the quirks that cause them.
- [x] Hunts run the rule.
- [x] TESTING.md has a section on invariants.
- [x] `scripts/check.sh` passes.

## Comments

**2026-10-16, done.**
- **The engine interface** (`examples/fuzz/engine.rs`): one-shot results, streams over given chunks, merges and conversions back to source, in batches, as JSON text. The core runs in-process; the reference runs through `serve.ts`, which learned two operations, full one-shot results (`parse`) and full streams (`stream`). Reading the reference's answers with `parse_json` matters: a result can hold a lone surrogate, which `serde_json` refuses.
- **The rule** (`examples/fuzz/invariants.rs`) is written once, run on both, and sorted. `--invariants` runs it; hunts give it the last quarter of their time, and their report has a row per rule.
- **A million programs** (seed 61, about a minute): 169,773 hold, 824,291 don't apply, 5,936 are exceptions, none unexplained, no mismatch.
- **What sorting turned up:**
  - **Narrowings, the rule being too strict:** it applies only to complete programs; a null prop equals a missing one; and it doesn't apply to trees referring to queries or mutations, trees with a cycle cut, or numbers JSON can't hold. The spec's section 9.2 backs the first three ("Trailing nulls for optional props are trimmed", "Query and mutation statements do not round-trip").
  - **New quirk 13:** serializing drops the parentheses operands need. `!($a && $b)` comes back as `!$a && $b`, `-($x + 1)` as `-$x + 1`, `-(3)` as the literal `-3`, and a ternary inside `||` or a member access on an expression lose theirs too. Section 9.2 says serializers should add them. `hand/107` pins it.
  - **New quirk 14, a candidate:** a call parsing made null is the expression `{k: "Null"}`, so a required prop it fills isn't checked, while a literal `null` there drops the component. `hand/108` pins it.
  - **Quirk 10 is by design since 1.0:** section 9.2 says object keys are written unquoted, so they must be valid names. Its draft report is withdrawn.
  - **Exceptions, each listed beside its quirk:** 13, 9, 10, 12, 14, 2, and the node-shaped literals.
- **A core fix,** found by the test hunt's differential fuzzing: a library call's validated argument replaced the written one whenever they differed structurally, and an object holding `NaN` never equals itself. So it was rebuilt, losing a duplicate key, where the reference keeps it by identity. `fuzz/106fb3f3` pins it.

**For the maintainer to judge:**
1. The narrowings above: right to narrow, or is any of them the reference being wrong?
2. Quirk 14: is it the reference being wrong (as written up), or covered by the spec's "up to the recovery rules of section 8", which would make it a narrowing?
3. Quirk 10: is withdrawing its draft report right?

**2026-11-09, the maintainer's judgments:**
1. **The narrowings are right**, all five: complete programs only; a null prop equals a missing one; no queries or mutations; no tree with a cycle cut; no numbers JSON can't hold.
2. **Quirk 14 is the reference being wrong**, not a narrowing. QUIRKS.md says so, and its upstream report stays a draft for the maintainer to file, or not.
3. **Withdrawing quirk 10's draft report is right:** the specification makes unquoted keys a design choice.
