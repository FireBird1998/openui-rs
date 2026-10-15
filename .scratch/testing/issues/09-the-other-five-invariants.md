# 09 — The other five invariants

**What to build:** The remaining rules, each written once against the engine interface from ticket 08 and run on both parsers:
1. **Chunking doesn't matter:** the final result is the same under every chunking.
2. **Streaming ends where one-shot parsing ends.** The corpus test already knows three kinds of programs where the reference breaks this.
3. **Fences don't matter:** a program inside a markdown fence, with prose before and after, parses the same as the bare program.
4. **Merging behaves:**
   - applying a patch twice gives the same as applying it once;
   - an empty patch changes nothing;
   - source converted back from a tree works as a patch.
5. **Finished statements stay put:** during streaming, a statement that is complete and fully resolved doesn't change in later chunks unless it's redefined. This ticket pins down "complete and fully resolved" precisely first.

Each rule runs on a million generated programs. Failures are sorted as in ticket 08, and each rule's exceptions are listed beside their quirks. Hunts run all five.

**Blocked by:** 08 — Round trip on both parsers

**Status:** ready-for-agent

- [ ] All five rules run on both parsers.
- [ ] "Complete and fully resolved" is defined in writing before rule 5 is checked.
- [ ] Each rule has run on a million programs, and every failure is sorted.
- [ ] Any new quirks are in QUIRKS.md with draft upstream reports, which aren't filed.
- [ ] Each rule's exceptions are listed.
- [ ] Hunts run all five.
- [ ] `scripts/check.sh` passes.
