# Spec: the viewer, a Mac app that draws OpenUI Lang as it streams

**Status:** ready-for-agent. Drafted 2026-10-15 from a design discussion. Each ticket starts only after the maintainer approves it. Comes before the testing phase (`../testing/spec.md`) and before the Dynamic News map comes off the shelf (`../dynamic-news/map.md`).

## Problem Statement

The core is finished and conforms to the reference, but I've only ever seen its output as JSON in a terminal. I can't watch a program draw itself chunk by chunk, see which parts are still arriving, or spot at a glance what's wrong with a model's output.

Dynamic News will also need the core behind a binding, drawn by a native UI that redraws on every chunk. Nobody has built that shape yet. The open questions in Dynamic News tickets 01 and 06 (how to build the binding, what crosses the boundary after each chunk, what that costs) would be slow to answer for the first time on an Android emulator.

## Solution

The **viewer**: a small SwiftUI app for the maintainer's Mac that calls the core through UniFFI. It plays a program as a stream and draws it natively, with an inspector beside the drawing showing the parse result.

It does two jobs:
- **A playground for the core.** Open any corpus case or program file, play it with any chunking, step and scrub through the chunks, and see the parse result's details at every step.
- **A rehearsal for Dynamic News.** It has the same shape as the Android app: core, binding, native UI. It measures what each chunk costs to cross the boundary and redraw, tries one alternative to sending JSON, and writes the answer into Dynamic News tickets 01 and 06.

It comes in two milestones:
1. **Offline:** the corpus and program files, the inspector, native drawing for the most-used components, and the boundary experiment.
2. **Live:** request a screen from a model, watch it stream and draw, record the stream with its real chunk boundaries, replay recordings, and follow up in edit mode. The testing phase imports these recordings into the corpus.

## User Stories

1. As the maintainer, I want to open any of the corpus cases with its component library, so that I can see what any program in the corpus looks like.
2. As the maintainer, I want to open a program file and pick a component library, so that I can look at programs outside the corpus.
3. As the maintainer, I want to paste a program, so that I can try something quickly without saving a file.
4. As the maintainer, I want the program drawn natively, so that I see what a user of a native app would see.
5. As the maintainer, I want the program played as a stream with a chunk size I choose, so that I see how it draws while it arrives.
6. As the maintainer, I want to play, pause, step forward and step back one chunk at a time, so that I can watch exactly what each chunk changes.
7. As the maintainer, I want a slider to jump to any chunk, so that I can go straight to the step I care about.
8. As the maintainer, I want to set the playback speed, so that I can watch fast streams slowly.
9. As the maintainer, I want elements that are still arriving to look different, so that I can tell partial elements from finished ones.
10. As the maintainer, I want any element the viewer can't draw natively shown as a labeled box with its props, so that every program draws something and nothing is hidden.
11. As the maintainer, I want the most-used components (text, stacks, cards, buttons, tables, form fields, tabs, tags, callouts and charts) drawn as real native controls, so that a screen looks like a screen.
12. As the maintainer, I want charts drawn with the system's chart framework, so that chart-heavy programs look right.
13. As the maintainer, I want an inspector showing the element tree at the current step, so that I can relate the drawing to the parse result.
14. As the maintainer, I want the inspector to list what's incomplete, unresolved and orphaned, so that I can see why a screen looks broken.
15. As the maintainer, I want validation errors listed with their component, path and statement, so that I can see what the model got wrong.
16. As the maintainer, I want the depth-limit and size-limit flags shown, so that I know when a screen was cut short.
17. As the maintainer, I want selecting an element in the drawing to select it in the inspector, and the other way round, so that I can find things in either view.
18. As the maintainer, I want the raw program text shown with the part received so far highlighted, so that I can see what the parser has seen.
19. As the maintainer, I want the inspector to say whether the current step matches the reference's golden for corpus cases, so that the playground also shows conformance.
20. As the maintainer, I want parsing to run off the UI thread, so that the viewer never freezes on a large program.
21. As the maintainer, I want the time each chunk takes shown (parsing, crossing to Swift, decoding and redrawing), so that I can see where the time goes.
22. As a Dynamic News developer, I want the cost of sending the whole parse result as JSON after every chunk measured on the largest corpus programs, so that we have a baseline for the Android binding.
23. As a Dynamic News developer, I want one alternative (a handle the UI walks, or only what changed since the last chunk) built and measured against that baseline, so that ticket 06 is decided on evidence.
24. As a Dynamic News developer, I want what we learn about the build setup, the binding and threads written into tickets 01 and 06, so that the Android work starts from it.
25. As the maintainer, I want to enter my Anthropic API key in the viewer's settings and have it kept in the macOS Keychain, so that it's never stored in a file or seen by an agent session.
26. As the maintainer, I want to type a request, pick a model and a component library, and watch the model's answer stream and draw, so that I see the full Dynamic News loop on my Mac.
27. As the maintainer, I want the system prompt generated by the core for the chosen library, so that the live loop uses the same prompt the web would.
28. As the maintainer, I want to see each live stream's token usage and estimated cost, and to set a budget cap that stops further requests, so that a mistake can't run up a bill.
29. As the maintainer, I want to save a live stream with its request, model and the chunks exactly as they arrived, so that it can be replayed and added to the corpus.
30. As the maintainer, I want to open a saved recording and replay it with its real chunk boundaries, so that I can watch it again without paying for it.
31. As the maintainer, I want to follow up on a screen with an edit request, have the model answer with a patch, and see the merged program drawn, so that edit mode is part of the loop.
32. As the maintainer, I want the patch shown beside the merged program, so that I can see what the model changed.
33. As the maintainer, I want to build and run the viewer with ⌘R in Xcode or with one command in the terminal, so that it fits however I'm working.
34. As the maintainer, I want the Xcode project reproducible from text files in the repository, so that an agent can change it safely and nothing depends on hand edits in Xcode.
35. As the maintainer, I want the binding's output checked against the core's for every corpus case at every streaming step, so that the viewer can't show something the core didn't produce.
36. As the maintainer, I want every corpus case decoded and drawn offscreen by the viewer's tests, so that no corpus program can crash it.
37. As a future Android developer, I want the binding crate written so that Kotlin can use the same one, so that the Android binding starts from what the viewer proved.

## Implementation Decisions

### Shape

- **A new binding crate** in the workspace wraps the core with UniFFI, using proc-macros, with no separate interface file. Swift uses it now and Kotlin later. The core crate gets no binding code.
- **Built optimized:** the binding crate is built at opt-level 1 or more in every profile, debug included, as the core's stack notes require.
- **Library form:** a static library for Apple Silicon macOS, with UniFFI-generated Swift bindings. A build script builds the library and generates the bindings. The Xcode build calls the same script, so ⌘R and the terminal produce the same thing.
- **The Xcode side is a Swift package** (decided in ticket 01), which Xcode opens directly, with a shared scheme whose pre-action builds the binding first. Both are plain text, so no generator tool is needed. A script wraps the built executable in an app bundle with its own identity.
- **Platform:** macOS 27 only. Swift 6 with strict concurrency, SwiftUI. Runs locally, unsigned, not distributed.

### The binding's interface, first version

- Load a component library from a JSON Schema text.
- One-shot parse of a program, optionally with a root name, returning the parse result as JSON text.
- A stream session over a library and an optional root name:
  - push a chunk, returning the result as JSON text;
  - set the whole text so far, returning the same;
  - read the current result.
- Later tickets add: system prompt generation from a prompt spec, merging a patch, and whichever alternative the boundary experiment builds.
- Everything the core reports as an error becomes a Swift error. Nothing crosses as a panic.

### What crosses after each chunk

- **The baseline** is the JSON text the core already produces, byte-identical with the web. Swift decodes it into its own model of the parse result.
- **The experiment** was going to build one alternative after measuring the baseline: either a handle to the Rust-side tree that Swift walks, or only what changed since the last chunk. Ticket 05 was rescoped on 2026-10-15 because crossing cost under 0.2 ms against a redraw of about 90 ms. It profiled and fixed the redraw instead, and recommends sending JSON.
- **Timings:** parse, crossing, decoding and redrawing are each timed per chunk and shown in the inspector.
- **Where the findings go:** Dynamic News tickets 01 and 06 get the build setup, the threads calls land on, the stack each needs, and the timings.

### Threads

- One stream session per window, owned by a dedicated parsing thread with a 1 MB stack.
- The UI talks to it through Swift concurrency and never parses on the main thread.

### Drawing

- A registry maps component names to native views.
- **Fallback:** any name the registry doesn't have is drawn as a labeled box showing the element's type and props, with its children drawn inside it.
- **Native components:** about 15 of the most used in upstream's example library, chosen from corpus usage on 2026-10-15:
  - text content, stack, card and card header;
  - button and button group;
  - table with its columns;
  - form, form control, input, and select with its items;
  - line, bar and pie charts with their series and slices, drawn with Swift Charts;
  - tabs with their items, tag and callout.

  Child parts (columns, select items, series, slices, tab items) are drawn by their parent.
- **Partial elements** are drawn as they are, with a subtle "still arriving" marker.
- Interactive controls look real but do nothing: there's no state, no queries and no actions, because the core doesn't evaluate them.

### Sources of chunks

Playback is driven by one interface, a source of chunks, with three implementations:
- **Corpus case or file, replayed:** with a chunk size and speed chosen in the viewer.
- **Recording, replayed:** with the boundaries and timing it was recorded with.
- **Live model stream.**

The drawing and the inspector don't know which one they're getting.

### Golden match

For corpus cases played with a chunking that has a golden, the inspector shows whether the current step's fingerprint matches the golden. With any other chunking it says there's no golden to compare with. The viewer reads the conformance folder of the repository it was built from, and settings can point it elsewhere.

### Live streaming (second milestone)

- **Provider:** Anthropic's Messages API with streaming. Models: Claude Sonnet 5.5 and Claude Haiku 5.5. Model ids and the streaming format are checked against the Claude API reference when the ticket starts.
- **System prompt:** generated by the core from the chosen component library. Edit mode uses the prompt's edit-mode option.
- **API key:** entered in the viewer's settings and stored in the macOS Keychain. It's never written to a file, logged, or shown back in full.
- **Cost:** each stream shows its token usage and estimated cost. A budget cap set in settings stops new requests once it's reached.
- **Recording:** a recording holds the request, the model, the component library's name, the time it was made, and every text delta exactly as received, with its arrival time. The testing phase defines how a recording becomes a corpus case and its recorded chunking. Recordings are saved to a folder chosen in settings.
- **Edit mode:**
  - a follow-up request asks the model for a patch;
  - the core merges it into the current program, and the viewer plays the merged program;
  - the patch is shown beside the merged program.

## Testing Decisions

- **What a good test checks:** behaviour at the outside of each piece. The binding returns what the core returns. Every corpus result decodes and draws. A source replays its chunks in order. No test checks view internals or pixel output.
- **Binding conformance (Rust, part of `scripts/check.sh`):** for every corpus case, one-shot and at every step of a chunking, the binding's JSON equals the core's. It runs anywhere Rust runs and needs no Swift.
- **Viewer tests (Swift, a separate check script, because they need Xcode):**
  - every corpus case's golden one-shot result decodes into the Swift model;
  - every corpus case's final result draws offscreen without crashing;
  - each source of chunks replays a known program in the right order with the right boundaries.
- **Not tested automatically:** the live model stream. Recordings replace it in tests.
- **Prior art:** the core's conformance test and corpus (cases, chunkings, goldens and fingerprints), and the CLI's stream command, which already prints a result per chunk.

## Out of Scope

- A Mac version of Dynamic News, or any news features.
- iOS, signing, notarization, and distribution of any kind.
- Native drawing for all 53 components in upstream's example library. The rest use the fallback box.
- Making controls work: state, queries, mutations and actions.
- Comparing against the live reference parser inside the viewer. Goldens cover conformance.
- Model providers other than Anthropic.
- A program editor beyond pasting text.
- Screenshot or pixel comparison tests.

## Further Notes

- **Toolchain on 2026-10-15:** Xcode 27.0, Swift 6.4 and macOS 27.0.1. The Xcode license had to be accepted again on 2026-10-14 before anything would link.
- **Corpus usage on 2026-10-15:** the most-used components in imported programs were text content (102 uses), stack (96), card (69), table column (62), button (60), select item (50) and card header (49). Upstream's example library has 53 components; 125 imported cases use it.
- **The core's needs from embedders:** opt-level 1 or more, and threads with at least 512 KB of stack. The viewer uses 1 MB.
- **Related:** Dynamic News tickets 01 (UniFFI on Android) and 06 (what the core hands Kotlin). The testing spec's recorded-stream tickets import the viewer's recordings.
