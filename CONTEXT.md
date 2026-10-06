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

### Correctness

**Reference parser**:
The TypeScript parser in `@openuidev/lang-core` at the pinned upstream commit; it defines correct behaviour.
_Avoid_: TS version, original, upstream parser

**Conformance**:
The core producing exactly the same serialized parse result as the reference parser for the same program, component library and chunking.
_Avoid_: parity, compatibility, equivalence

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
