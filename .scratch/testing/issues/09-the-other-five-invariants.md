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

**Status:** done (2026-11-09).

- [x] All five rules run on both parsers.
- [x] "Complete and fully resolved" is defined in writing before rule 5 is checked.
- [x] Each rule has run on a million programs, and every failure is sorted.
- [x] Any new quirks are in QUIRKS.md with draft upstream reports, which aren't filed.
- [x] Each rule's exceptions are listed.
- [x] Hunts run all five.
- [x] `scripts/check.sh` passes.

## Results

Checked 2026-11-09.
- **Rules:** chunking, stream end, fences, merging twice, merging nothing, merging a program's own source and finished statements stay put (3019307). They run on both parsers beside ticket 08's round trip.
- "Complete and fully resolved" is defined in TESTING.md, and each rule's exceptions are listed there by quirk.
- **New quirks:** 15–23 (some later fixed upstream) are in QUIRKS.md. The upstream report drafts are GitHub #20 and #21; none is filed.
- **Hunts:** they run the invariants.
- **A million programs per rule:**
  - each long run on the worker Mac (2026-11-08 at 185735a, re-run at be4b5d3) ran every rule over two 500,000-program ranges;
  - the re-run's tallies had 0 mismatches and 1 case broken on both;
  - that case was quirk 23's shape, recognized since be4b5d3; every other failure is explained by a listed quirk or divergence.
- `scripts/check.sh` passes.
