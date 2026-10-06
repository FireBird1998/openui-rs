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
The set of components a program may use, each with its props in positional order, supplied as JSON Schema.
_Avoid_: catalog, schema, registry

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
A way of cutting a program into pushes for a streaming parse: a fixed size in characters, or seeded random sizes.
_Avoid_: split, batching

**Golden**:
The reference parser's saved output for a case: the one-shot result, and a fingerprint of every streaming step under every chunking.
_Avoid_: snapshot, expected output

**Pending case**:
A case the core doesn't conform on yet, listed so the suite stays green while showing progress.
_Avoid_: skipped, known failure

**Quirk**:
A reference-parser behaviour that looks wrong but that the core reproduces anyway, recorded with an upstream report.
_Avoid_: bug (when the core matches it on purpose)

### Products

**Core**:
The `openui-lang` Rust crate: parsing, streaming parsing, validation and prompt generation. Not evaluation, state or queries.
_Avoid_: runtime, engine, SDK

**Dynamic News**:
The showcase Android app whose story screens are written by an LLM, parsed by the core on the phone, and drawn with Jetpack Compose.

**Story screen**:
The screen for one news story, whose layout is a program generated for that story.
_Avoid_: article page, detail view
