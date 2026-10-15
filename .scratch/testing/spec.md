# Spec: testing openui-lang thoroughly, and keeping it tested

**Status:** ready-for-agent. Drafted 2026-10-14 from a grilling session. Each ticket starts only after the maintainer approves it. On 2026-10-15 this phase was put after the Mac viewer (see the Dynamic News map); the ticket breakdown is drafted but not yet approved or written.

## Problem Statement

The core conforms to the reference parser on a large corpus at every streaming step, and no fuzzed input has ever crashed it. But I can't yet say it's thoroughly tested, or keep saying so as it changes:

- Nobody has measured which code no test runs, or whether the tests would notice a bug planted in the code.
- The coverage-guided fuzz targets have never run, and there are none for merging, converting back to source or reading JSON.
- Every program in the corpus is hand-written, taken from upstream's docs, or generated. None is a real model's output streamed with the chunk boundaries a real API produces, which is exactly what Dynamic News will feed the core.
- Comparing with the reference can't find bugs the reference shares. Quirks have only turned up by accident, though each one is a bug in OpenUI itself.
- Everything has run on one Mac. Dynamic News runs the core on Android phones, with smaller thread stacks, where nothing has been tried.
- Bug hunting happens only when someone remembers to run the fuzzer, and nobody notices when upstream changes.

I want to know how to test the core and where it needs testing, to reach a point where it's thoroughly tested, and to have hunts keep looking for bugs after that.

## Solution

A testing phase with a defined finish line, after which hunts keep going on their own.

The phase:
- measures coverage and sorts out every line no test runs;
- plants bugs (mutation testing) and sorts out every one the suite misses;
- fuzzes every public entry point with coverage guidance, counting inputs that take far too long as findings;
- checks six invariants on the core and the reference alike, so a rule the reference breaks surfaces as a new quirk instead of staying invisible;
- adds real model streams to the corpus, with the chunk boundaries the API actually sent;
- runs the suite on the Android emulator, and once on a real phone if one is available;
- builds a hunt that runs on demand and later nightly, leaves findings and a report, never commits, and reports upstream changes;
- explains all of it in TESTING.md: how to run each kind of test, add a case, read a failure and sort a finding.

**Finish line.** The core counts as thoroughly tested when:
- every line no test runs is covered by a new case, deleted as dead, or explained;
- every planted bug the suite misses is caught by a new case, or explained as changing nothing;
- every coverage-guided fuzz target has run a full night (about 8 hours) and found nothing new by the end;
- every invariant holds on a million generated programs, apart from listed exceptions;
- about 50 recorded model streams conform;
- the whole suite passes on the 64-bit ARM Android emulator, including the stack tests at phone-sized thread stacks;
- one run on a real phone has passed, if the maintainer has one;
- the hunt and TESTING.md exist.

## User Stories

1. As the maintainer, I want a report of every line no test runs, so that I know where the suite is blind.
2. As the maintainer, I want each unrun line either covered by a new case, deleted as dead code, or listed with a reason, so that every blind spot is a decision rather than an accident.
3. As the maintainer, I want to re-run the coverage report with one command, so that I can see whether new code is tested.
4. As the maintainer, I want bugs planted in the code automatically and the suite run against each one, so that I know whether the tests would catch a real mistake.
5. As the maintainer, I want every planted bug the suite misses to be caught by a new case or explained as changing nothing, so that weak tests get fixed.
6. As the maintainer, I want the explanations for unrun lines and missed planted bugs kept in the repository, so that later runs don't make me sort them out again.
7. As the maintainer, I want mutation runs not to call Bun, so that an overnight run finishes overnight.
8. As the maintainer, I want mutation testing to run overnight on AC power and leave a report, so that it doesn't tie up my laptop while I work.
9. As the maintainer, I want coverage-guided fuzzing of one-shot and streaming parsing, so that crashes in rarely reached code are found.
10. As the maintainer, I want coverage-guided fuzz targets for merging, converting back to source and reading JSON, so that every public entry point is fuzzed, not just parsing.
11. As the maintainer, I want an input that takes far too long to count as a finding, so that blow-ups are caught as well as crashes.
12. As the maintainer, I want coverage-guided fuzzing to start from the corpus, so that it explores near real programs from the first minute.
13. As the maintainer, I want every crash and blow-up shrunk to a small input and saved as a case, so that it stays fixed.
14. As the maintainer, I want "parse, convert back to source, parse again gives the same tree" checked on generated programs, so that the round trip upstream's own tests promise holds beyond their three examples.
15. As an embedder, I want "the final result doesn't depend on chunking" checked, so that a reader sees the same screen however the network split the stream.
16. As the maintainer, I want "streaming ends where one-shot parsing ends" checked on generated programs, not just on the corpus.
17. As an embedder, I want "a program inside a markdown fence, with prose around it, parses the same as the bare program" checked, so that models that wrap their output still work.
18. As the maintainer, I want merging checked: a patch applied twice gives the same as applying it once, an empty patch changes nothing, and source converted back from a tree works as a patch.
19. As an embedder, I want "a statement that is complete and fully resolved doesn't change in later chunks unless it's redefined" checked, so that story screens don't flicker while streaming.
20. As the maintainer, I want each invariant checked on the reference too, so that a rule the reference breaks is recognized as a quirk instead of reported as a core bug.
21. As the maintainer, I want each invariant written once and run on both parsers, so that the two checks can't drift apart.
22. As the maintainer, I want each invariant's exceptions listed beside the quirk that causes them, so that the rule still guards everything else.
23. As an OpenUI maintainer, I want every new quirk recorded with a draft upstream report, so that bugs found here can reach OpenUI. Filing stays the maintainer's call.
24. As the maintainer, I want real model outputs in the corpus, so that the core is tested on what models actually write: truncation, fences, invented components and odd formatting.
25. As an embedder, I want each recorded stream checked with the chunk boundaries the API actually sent, so that conformance covers real network chunking, not just synthetic chunk sizes.
26. As the maintainer, I want to record streams myself with a script that reads my API keys from my shell, so that keys never pass through an agent session.
27. As the maintainer, I want recording to stop at a budget cap, so that a mistake can't run up a bill.
28. As the maintainer, I want edit-mode follow-ups recorded too, so that merging is tested on real patches.
29. As the maintainer, I want recorded streams checked offline like every other case, so that the suite still needs no Bun and no network.
30. As an embedder, I want the whole suite to pass on the 64-bit ARM Android emulator, so that I know the core works on the platform Dynamic News ships on.
31. As an embedder, I want the stack tests to pass on Android threads with phone-sized stacks, so that deep programs can't crash the app.
32. As the maintainer, I want to point the suite at a conformance directory anywhere, so that it can run where the source tree isn't, such as on a phone.
33. As the maintainer, I want one run on a real phone if I have one, so that the emulator isn't the only evidence.
34. As the maintainer, I want the build checked on the oldest Rust we claim to support, so that the claim stays true.
35. As an embedder, I want a compile-time guarantee that parsers and component libraries can move between threads and be shared across them, so that I can parse off the UI thread.
36. As the maintainer, I want one command that hunts for a set time and leaves shrunk cases and a report, so that I can look for bugs whenever I have an hour.
37. As the maintainer, I want hunts to run nightly on AC power once they're stable, so that bug hunting doesn't depend on my remembering.
38. As the maintainer, I want a hunt never to commit or change tracked files, so that nothing enters history without my review and nothing shifts under me while I work.
39. As the maintainer, I want each hunt's report to say what was found, what kind of finding it is and how to reproduce it, so that review takes minutes.
40. As the maintainer, I want hunts to tell me when upstream has new commits and what they change across our corpus, so that I can decide when to move the pin.
41. As the maintainer, I want TESTING.md to explain each kind of test, how to run it, how to add a case and how to read a failure, so that I can test the core myself.
42. As a contributor, I want TESTING.md to say which test catches which kind of bug, so that I know where a new test belongs.
43. As the maintainer, I want each finding sorted into a mismatch, a crash, a blow-up, a platform bug or a new quirk, so that each gets the right fix.
44. As the maintainer, I want the small fixes from a hunt in one commit after the check passes, and design questions as tickets, so that findings don't bury me in tickets.
45. As anyone reading the repository, I want CONFORMANCE.md to report coverage, mutation results, fuzzing hours, invariants and recorded streams, so that I can see how thoroughly the core is tested.

## Implementation Decisions

### Where the core is tested

- In this phase: the maintainer's Mac (Apple Silicon), and the Rust core alone on the 64-bit ARM Android emulator. An arm64 emulator image and device already exist and run natively on Apple Silicon.
- Once at the end: a real Android phone, if the maintainer has one.
- Elsewhere:
  - The Kotlin boundary (UniFFI) is tested in Dynamic News ticket 01, where the binding gets built.
  - A backend is tested only if Dynamic News ever decides to have one.
  - 32-bit Android is out of scope.

### Where tests attach

1. **The public API.** Every new kind of test drives the crate through what an embedder can call: one-shot and streaming parse, merge, converting back to source, prompts, error hints and JSON. No new test reaches into internal modules. The existing internal unit tests stay.
2. **The corpus of cases and goldens.** Real model streams join as ordinary cases with one addition, a **recorded chunking**: the chunk boundaries the API actually sent, stored with the case. The reference computes its golden like any other chunking's, and the offline suite checks it the same way.
3. **One rule, two parsers.** An engine interface (full one-shot result, stream over given chunks, merge, convert back to source) with two implementations: the core in-process, and the reference through the long-lived reference process the differential fuzzer already drives. That process learns to return full parse results; today it returns only fingerprints for parsing. Invariants are written once against the interface. The interface lives with the differential fuzzer, not in the crate's public API.
4. **Where the suite finds its data.** Two environment switches. One names the conformance directory; the default stays the repository's. The other turns off live reference diffs on failure. They let the suite run on a phone and under mutation testing.

### Sorting findings

- **Mismatch:** fixed in the core, with a case first.
- **Crash or blow-up:** fixed in the core. If the reference also blows up on the input and the input is unlike real output, it becomes a documented divergence, as ticket 14's exponential case did.
- **Platform bug:** fixed in the core.
- **Invariant broken by both parsers:** a candidate quirk. The maintainer judges whether the reference or the rule is wrong. If the reference is wrong, it becomes a quirk with a draft upstream report, and the rule gets an exception. If the rule is wrong, the rule is narrowed or dropped.
- **Invariant broken by the core only:** the core differs from the reference on that input, so it's a mismatch.
- **Review:** findings are reviewed with the maintainer, one hunt report at a time. Small fixes go in one commit after the check passes. Anything needing a design decision becomes a ticket.

### Tools

Each is installed only after the maintainer approves, when its ticket starts:
- cargo-llvm-cov, for coverage;
- cargo-mutants, for mutation testing;
- cargo-fuzz, for coverage-guided fuzzing, on the nightly toolchain that's already installed;
- cargo-ndk and the aarch64 Android Rust target, for Android.

### Coverage

- Measured over the offline suite, the same tests the check runs.
- There's no percentage target. The bar is that no unrun line is left unexplained.
- Lines that are unreachable by design are listed with a reason in a committed file.

### Mutation testing

- Runs on the test profile, with live diffs off.
- Runs overnight on AC power, split into two module groups:
  - parsing and streaming;
  - edits, prompts, error hints and JSON.
- Each mutant costs about 29 seconds: a rebuild plus the full suite, which takes about 20 seconds.
- Mutants that change nothing are listed with a reason in a committed exclusion file, so reruns skip them.

### Coverage-guided fuzzing

- Targets cover one-shot parse and streaming (both exist), plus merge, converting back to source, and reading JSON.
- Each target starts from the corpus and the existing dictionary.
- A per-input time limit turns slow inputs into blow-up findings. The limit is set in the ticket.
- Each target runs one full night. Targets run in parallel across cores.

### Invariants

1. **Round trip:** parse, convert back to source, and parse again gives the same tree, ignoring the partial and dynamic-prop flags the way upstream's own round-trip test does.
2. **Chunking doesn't matter:** the final result is the same under every chunking.
3. **Streaming ends where one-shot parsing ends.**
4. **Fences don't matter:** a program inside a markdown fence, with prose before and after, parses the same as the bare program.
5. **Merging behaves:**
   - applying a patch twice gives the same as applying it once;
   - an empty patch changes nothing;
   - source converted back from a tree works as a patch.
6. **Finished statements stay put:** during streaming, a statement that is complete and fully resolved doesn't change in later chunks unless it's redefined. The ticket pins down "complete and fully resolved" precisely.

Each invariant is checked on a million programs from the differential fuzzer's generator. Its exceptions are listed with the quirks that cause them.

### Recorded model streams

- **Recorded through the viewer (decided 2026-10-15):** the viewer streams from models and saves recordings, so this phase imports its recordings into the corpus instead of building a separate recording script.
- **Who runs it:** the maintainer, in the viewer. API keys live in the macOS Keychain, entered by the maintainer in the viewer's settings, and are never logged.
- **Budget:** a hard cap of $10.
- **Models:** Claude Sonnet 5.5 and Claude Haiku 5.5, plus a third provider if the maintainer has a key.
- **Requests:** 40 fresh screens and 10 edit-mode follow-ups. Claude writes the requests, about half of them news-shaped.
- **System prompts:** the core generates them, using upstream's example component libraries.
- **What's stored per stream:** the request, the model, and the text deltas exactly as received. The recorded chunking comes from those deltas.
- **Re-recording:** recording again with the news component library and the on-device model belongs to Dynamic News.

### Android

- **How:** test binaries are cross-compiled and pushed to the emulator with the conformance data, then run there. The stack tests run at phone-sized thread stacks; they already take their stack size from the environment.
- **When:** this run is on demand. It isn't part of the check, because it needs a running emulator, and hunts don't include it.

### Hunts

- **When:** on demand, with a time budget. Later, a scheduled job runs one nightly, only on AC power.
- **What a hunt runs:**
  - differential fuzzing with a fresh seed, for parsing and for edits;
  - coverage-guided fuzzing;
  - the invariants;
  - an upstream check.
- **Output:** a dated report and the shrunk cases, in a findings folder git ignores. A hunt never commits and never changes tracked files. A finding becomes a case during review.
- **Upstream check:** looks for new upstream lang-core commits. When there are some, it vendors the new reference into a scratch copy, computes goldens for our corpus there, and reports which cases' outputs changed. Moving the pin is a separate ticket.
- **Speed:** hunts don't track it, because laptop timings are too noisy. The streaming-cost test stays as the guard.

### Small additions to the check

- It also builds on Rust 1.88, the oldest version we claim to support.
- A compile-time assertion that the streaming parser, parse results and the component library can move between threads and be shared across them.

### Documentation

- TESTING.md is created in the first ticket. Each later ticket adds its own section.
- At the finish line, CONFORMANCE.md gains a section with the numbers: coverage, mutation results, fuzzing hours, invariants and recorded streams.

## Testing Decisions

- **What a good test checks:** behaviour through the public API, against an oracle: the reference's output (a golden), an invariant, or a resource bound. Never internal structure.
- **Every finding becomes a permanent case before it's fixed**, so the fix is proven and stays proven.
- **The check stays offline.** No Bun and no network. New cases and recorded streams come with their goldens committed.
- **Modules tested:** the whole public surface. Mutation testing and coverage look at every module.
- **Prior art:**
  - the conformance test: goldens, chunkings, diffs against the live reference;
  - the robustness property tests: no panic on any text;
  - the depth tests: stack size taken from the environment;
  - the streaming-cost test: thread CPU time, best of several runs;
  - the differential fuzzer: generation, mangling, shrinking, saving cases;
  - the existing cargo-fuzz targets and dictionary;
  - the report test, which keeps CONFORMANCE.md's counts honest.

## Out of Scope

- **Elsewhere in Dynamic News:**
  - the Kotlin/UniFFI boundary (Dynamic News ticket 01);
  - app-level tests, such as replaying recorded streams through Compose screens, or screenshot tests;
  - re-recording streams with the news component library and the on-device model.
- **Platforms:** a backend, 32-bit Android, and iOS.
- **Tools and process:**
  - Miri: the crate has no unsafe code;
  - tracking speed regressions in hunts;
  - CI on GitHub. There's no remote. If one is added, CI runs only the fast offline suite.
- **Maintainer's calls:**
  - filing upstream reports;
  - moving the upstream pin, which is its own ticket when hunts report changes.

## Further Notes

- **Measured on 2026-10-14:**
  - the offline suite takes about 20 seconds, and one conformance test is about 40% of that;
  - each planted bug costs about 29 seconds;
  - the crate's source is about 7,600 lines.
- **Linking on macOS:** it failed on 2026-10-14 until the Xcode license was accepted again. A macOS or Xcode update can reset it. The symptom is a linker error about the Xcode license agreements.
- **Glossary:** this spec added Divergence, Mismatch, Invariant, Hunt and Finding, and added recorded chunkings to Chunking.
- **Related:** the Dynamic News map, tickets 01 (UniFFI on Android) and 06 (what the core hands Kotlin).
