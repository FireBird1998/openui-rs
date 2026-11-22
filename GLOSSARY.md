# openui-rs

A Rust implementation of the OpenUI Lang language layer, and Dynamic News, the Android app that is its first real user.

## Language

### The language

**OpenUI Lang**:
The compact, line-based language an LLM writes to describe a user interface.
_Avoid_: DSL, markup, UI JSON

**Program**:
A piece of OpenUI Lang text: a sequence of statements, possibly still arriving.
_Avoid_: document, snippet, response

**Statement**:
One `name = expression` line of a program; its name is the statement id.
_Avoid_: line, assignment, declaration

**Component library**:
The set of components a program may use, each with its props in positional order, supplied as JSON Schema or built from a library definition.
_Avoid_: catalog, schema, registry

**Library definition**:
The declarations of a library's components, functions and actions, from which the core builds the component library's JSON Schema and its prompt spec.
_Avoid_: registry, catalog, builder

**Element**:
One component instance in the parsed tree, with its positional arguments mapped to named props.
_Avoid_: node, widget, component (when meaning an instance)

**Parse result**:
What parsing returns: the root element plus meta (incomplete, unresolved, orphaned, errors), state declarations, and query and mutation statements.
_Avoid_: AST, output, tree (when meaning the whole result)

**Streaming parse**:
Parsing a program chunk by chunk as it arrives, producing a parse result after every chunk.
_Avoid_: incremental render, live parse

**Patch**:
A program holding only the statements that changed, merged into an existing program by statement id; `name = null` deletes. Edit mode is the model writing patches instead of whole programs.
_Avoid_: diff, delta, update

**Prompt spec**:
The JSON that describes what goes into a system prompt: component signatures and groups, tools, examples, rules and mode flags; the reference's `PromptSpec`, or `{library, promptOptions}`.
_Avoid_: prompt config, prompt options (when meaning the whole input)

**System prompt**:
The text generated from a prompt spec that tells the model how to write OpenUI Lang for a given component library.
_Avoid_: instructions, preamble (that's one part of it)

**Cloud config block**:
What upstream's `generateSystemPrompt({cloud: true, ...})` gives instead of a system prompt, for OpenUI Cloud to build the prompt from: a `]]>openui:config` marker line and JSON naming the component library (without its components) and the only prompt options Cloud carries (examples, preamble and additional rules), or Cloud's built-in library. Sending it as the system prompt is cloud prompt mode; sending the system prompt itself is full prompt mode.
_Avoid_: cloud prompt, cloud system prompt (for the block itself)

**Message**:
A response as stored or streamed, with marker lines around its parts: content, context, attributes and an end.
_Avoid_: envelope, frame, payload

**Marker line**:
A line starting `]]>openui:` that begins a part of a message.
_Avoid_: tag, delimiter, header

**Response bundle**:
A message whose content carries a scripts section after the program, as OpenUI Cloud writes it.
_Avoid_: package, Cloud response

**Response reader**:
The core's reader for a raw response handed over whole as it grows: it remembers how far it read, goes through only what an append adds, and reads any other edit afresh, with the same result (ADR 0015).
_Avoid_: incremental parser, delta reader

**Cloud script**:
A named piece of code in a response bundle that a query calls like a tool; it runs in OpenUI Cloud.
_Avoid_: function, server tool, script (alone)

### Evaluation

**Store**:
State values held by name, with declared starting values, pristine status and a version that rises when state changes.
_Avoid_: state manager, model, cache

**Evaluated tree**:
The element tree after expressions have been resolved against the store, with bindings marked and actions kept as plans.
_Avoid_: rendered tree, parse result (when meaning the evaluated tree)

**Binding**:
A component prop that reads and writes a state variable, optionally transforming a written value with an expression.
_Avoid_: reactive prop, link

**Binding marker**:
A value naming the state variable a binding writes and the expression that transforms a written value.
_Avoid_: reactive assignment, binding object

**Action plan**:
An ordered list of steps kept in an evaluated tree and run when the embedder asks, such as after a click.
_Avoid_: action object, command list, handler

**Step**:
One operation in an action plan: set, reset, run, to assistant, open URL or custom action. A set step keeps its value expression until the plan runs.
_Avoid_: command, instruction, action (when meaning one step)

**Host event**:
A step the runtime returns for the embedder to perform, with its checked parameters and form fields.
_Avoid_: callback, effect, notification

**Query manager**:
The part of the runtime that runs a program's queries and mutations and holds their results.
_Avoid_: fetcher, data layer, scheduler

**Tool provider**:
What an embedder attaches to a session to perform tool calls.
_Avoid_: backend, client, handler

**Tool call**:
One request to a tool, with its name and arguments, and its outcome: a result or a failure.
_Avoid_: fetch, request, invocation

**Tool-reported error**:
A tool call that ran and answered with an error, as an MCP response with `isError` does. The MCP result helper calls it `ToolReported`, Swift's provider `toolReported`; the query manager reports it as `mcp-error`.
_Avoid_: tool error (that is a call failure)

**Call failure**:
A tool call that did not produce an answer: the provider threw, was cancelled, or its result was refused under a limit. The query manager reports it as `tool-error`.
_Avoid_: tool-reported error, MCP error

**Refresh interval**:
How often a query is fetched again, in seconds.
_Avoid_: polling rate, timer, TTL

**Blocked**:
A response that has not settled: still streaming or, for a response bundle, incomplete or with an invalid scripts section. Upstream draws buttons and inputs disabled while a response is blocked; a blocked response bundle also ignores clicks and Retry.
_Avoid_: unsettled, pending, locked

**Settled**:
A program whose response has finished streaming and, for a response bundle, is complete and valid. Queries and mutations run only for a settled program.
_Avoid_: unblocked, done, complete (a parse result's own flag)

**Pristine**:
A state name still holding a declared starting value. A later parse may replace that value; a value set by an app or a user is no longer pristine.
_Avoid_: untouched, clean, initial (when meaning pristine status)

### Correctness

**Reference parser**:
The TypeScript parser in `@openuidev/lang-core` at the pinned upstream commit; it defines correct behaviour.
_Avoid_: TS version, original, upstream parser

**Conformance**:
The core producing exactly the same serialized parse result as the reference parser for the same program, component library and chunking.
_Avoid_: parity, compatibility, equivalence

**Case**:
One program plus the component library (and optional root name) it's parsed against; the unit of conformance.
_Avoid_: fixture, sample, test

**Chunking**:
A way of cutting a program into pushes for a streaming parse: a fixed size in characters, seeded random sizes, or the boundaries a model's stream actually had (a recorded chunking).
_Avoid_: split, batching

**Golden**:
The reference parser's saved output for a case: the one-shot result, and a fingerprint of every streaming step under every chunking.
_Avoid_: snapshot, expected output

**Pending case**:
A case the core doesn't conform on yet, listed so the suite stays green while showing progress.
_Avoid_: skipped, known failure

**Quirk**:
A reference-parser behaviour that looks wrong but that the core reproduces anyway, recorded with a draft upstream report unless it only follows from an upstream fix. Comparing the two can't reveal one; only invariants and people reading outputs do.
_Avoid_: bug (when the core matches it on purpose), shared bug

**Divergence**:
A documented place where the core or the runtime departs from the reference on purpose: on inputs no real program produces, or where the reference's behaviour is a defect we chose not to copy, recorded with a draft upstream report.
_Avoid_: deviation, exception

**Mismatch**:
A difference between the core's output and the reference parser's that no divergence explains.
_Avoid_: diff, regression, failure

**Invariant**:
A rule every program should satisfy, such as "parsing, converting back to source and parsing again gives the same tree", checked on the core and the reference alike; where the reference breaks one, the exception is a quirk.
_Avoid_: property, law

### Limits

**Limit**:
A documented bound on what one parse may read, build or do: program size, result size, depth or work.
_Avoid_: cap, quota, budget (when meaning the bound itself)

**Cut result**:
A parse result in which a limit was hit, or a stall stopped the parse in a release build, flagged as such, with what lay past that point left out.
_Avoid_: truncated result, partial result, limited result

**Amplification**:
How many times larger a parse result is than its program.
_Avoid_: blow-up factor, expansion

**Stall**:
A parsing loop consuming input without advancing through it; always a bug in the core, never a property of the input.
_Avoid_: hang, infinite loop, livelock

### Hunting

**Hunt**:
A time-boxed search for findings, run on demand or on a schedule, that leaves shrunk cases and a report for review and never commits.
_Avoid_: fuzz run, nightly, sweep

**Finding**:
Anything a hunt or test turns up before it's sorted: a mismatch, a crash, a blow-up (too slow or too much memory), a platform bug (right on the Mac, wrong on a phone) or a new quirk.
_Avoid_: bug report, issue

### Products

**Core**:
The `openui-lang` Rust crate: parsing, streaming parsing, validation and prompt generation. It evaluates a state variable's starting value while parsing; it keeps no live state and runs no queries.
For live state, actions and queries, see **Runtime**.
_Avoid_: engine, SDK

**Runtime**:
The `openui-runtime` Rust crate over the core's one evaluator: live state, evaluated trees, bindings, action plans and the query manager. It depends on the core; the core does not depend on it.
_Avoid_: engine, renderer, core (when meaning the runtime)

**Dynamic News**:
The showcase Android app whose story screens are written by an LLM, parsed by the core on the phone, and drawn with Jetpack Compose.

**OpenUINative**:
The Swift package that draws programs as native SwiftUI views, for any Mac app: a live program's evaluated tree, or a bare parse result. It holds the core's Swift binding.
_Avoid_: the Mac binding (the binding is one part of it), UI kit, renderer

**Intelligent UI library**:
Upstream's Intelligent UI component library (52 components in six groups under `Col`, and 59 functions) written in Swift in the `OpenUIIntelligentUI` target beside OpenUINative: what the model sees is an exact copy of upstream's, with two added chart types as the only difference, and the views are native SwiftUI.
_Avoid_: the Intelligent UI kit, the native library, the Swift library (OpenUINative is Swift too)

**Live program**:
In OpenUINative, a program kept live by a runtime session (`LiveProgram`): the app sets and reads state, `ElementView` draws the evaluated tree and redraws when the store changes, built-in buttons run their action plans (a primary button in a form first checks its fields' rules, as upstream's does), built-in inputs write through binding markers or form fields, and one handler receives the host events. A click or a keystroke acts only while what it was drawn from is still what the tree holds there. While the response on screen is blocked (the app says it is streaming, or the session says a response bundle has not settled), buttons and inputs are disabled.
_Avoid_: state runtime (its name before the runtime existed), runtime (that's the crate)

**Viewer**:
The Mac app that plays a program as a stream, evaluates every step and draws it natively beside its store and parse result: a development tool for the core and the runtime, built on OpenUINative, and a rehearsal for Dynamic News's binding.
_Avoid_: playground, inspector, demo app, Mac app

**Chat app**:
The sandboxed Mac app in `tools/chat/` that sends a thread to the Thesys Gateway with the person's own key, kept in the Keychain, and draws each answer natively as it streams: a real product built on OpenUINative's public interface, to prove the Swift binding is complete.
_Avoid_: demo, Mac app (the Viewer is a Mac app too)

**Story screen**:
The screen for one news story, whose layout is a program generated for that story.
_Avoid_: article page, detail view
