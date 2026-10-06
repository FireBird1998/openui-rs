# 02 — Real-world corpus and the chunking matrix

**What to build:** Grow the conformance corpus from one program to everything we can get:
- the salvaged hand-written corpus with its validation schema
- the 7 benchmark samples and their schema
- inputs lifted from the reference test suites (parser, materialize validation)
- OpenUI Lang examples from the upstream docs and examples folders

Every case runs one-shot, at chunk sizes 1, 2, 3, 4, 7 and 16, and with seeded random sizes. Cases the core can't handle yet go on the pending list, so the suite stays green and prints "N of M cases conformant".

**Blocked by:** 01 — Walking skeleton.

**Status:** needs-triage

- [ ] Every corpus case has committed golden output from the reference parser for every chunking.
- [ ] The test run prints the count of conformant cases.
- [ ] A pending case that starts passing fails the suite until it's removed from the pending list.
- [ ] A case that used to pass and now fails fails the suite.
- [ ] Each case records where it came from (hand-written, benchmark, reference test, docs).
