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

**Status:** ready-for-agent

- [ ] The engine interface exists, with both implementations; the reference process returns full results.
- [ ] The round-trip rule is written once and runs on both.
- [ ] It has run on a million generated programs, and every failure is sorted.
- [ ] Any new quirk is in QUIRKS.md with a draft upstream report, which isn't filed.
- [ ] The rule's exceptions are listed beside the quirks that cause them.
- [ ] Hunts run the rule.
- [ ] TESTING.md has a section on invariants.
- [ ] `scripts/check.sh` passes.
