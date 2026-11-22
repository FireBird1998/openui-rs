---
status: accepted
---

# Library definition lives where upstream puts it, and a library owns its own functions

Decided by the maintainer on 2026-11-21, while reworking the draft spec in
#123. Nothing here is built yet; #123 is the spec and its tickets carry the
work. This record exists so a later change knows what was decided, what was
rejected and why.

## What happened

Until now a component library could only reach the core as ready-made JSON:
the JSON Schema and prompt spec that upstream's JavaScript tooling prints.
`CONFORMANCE.md` listed upstream's authoring API (`defineComponent`,
`createLibrary` and the zod signature helpers) under "Deliberately not ported
from lang-core".

A first draft for a native library proposed adding the Intelligent UI
library's 59 functions to the Rust parser as built-ins. The maintainer
rejected that. The parser decides whether a name is a call or a component from
one fixed table of 20 built-ins. Adding names to it would stop a component of
the same name from parsing as a component, would leave those functions'
arguments unvalidated (built-ins are not validated), and would make the parser
diverge from lang-core 1.1.0. Library functions already have their own path:
the schema's `functions`, called with `@`, validated, with the implementation
registered by a host.

## The rule

Copy upstream, and put each thing where upstream's owner of it sits.

- What **lang-core** owns goes in the Rust core. `defineComponent`,
  `defineFunction`, `defineAction` and `createLibrary`, and the code that
  turns a library into its JSON Schema and prompt spec, go in the `openui-lang`
  crate as their own modules beside the parser, as upstream keeps them in
  their own files outside its `parser` folder. The parser's own files do not
  change.
- What **the renderer** (upstream's react-lang) owns goes in the host binding:
  pairing a component name with its view, and handing a library's function
  implementations to the evaluator when a program is shown.
- What a **library package** owns lives in that library, written in that
  library's host language. A Swift library defines its functions in Swift
  through `defineFunction`; a Kotlin library defines its own in Kotlin. No
  library's components or functions go in any Rust crate.

A separate Rust crate holding one library's functions, shared by every host,
was proposed and rejected. A library author adding one function should not
have to touch a Rust crate, and the library is the example of how a host
builds on the core.

## The parser guarantee

A library definition reaches the parser only as the JSON the parser already
reads. The spec's acceptance criteria hold this:

- no change to the lexer, parser, expressions, statements, built-ins or
  materializing;
- the built-in table stays at 20 names;
- the golden files do not change;
- a defined library and the same library supplied as JSON give the same parse
  results on the whole corpus.

## No zod

Upstream authors write declarations with zod, and lang-core asks zod for the
JSON Schema. Rust, Swift and Kotlin have no zod, and upstream itself has no
zod-free way to write a library, though its `spec/prompt.md` defines the
library as one JSON document and says each platform writes its components
natively. Upstream issues 393 (Swift) and 395 (Kotlin) ask for the same thing.

An experiment with upstream's real code showed that a plain description of
each prop is enough to reproduce upstream's output byte for byte, provided it
carries what JSON Schema alone loses: whether the prop is a binding, a display
name for a named type, optional and nullable and default as separate facts,
and the order of props and union members.

So:

- **The core takes plain data.** A recursive description of each prop's type,
  crossing the binding as JSON text like every other value.
- **A host author writes a small typed description** in the host language,
  which produces that plain data. Raw dictionaries were rejected as the
  author-facing form (nothing checks them until the app runs), deriving from
  native structs with macros was rejected (it cannot express bindings, named
  types or component references, and its tooling needs a licence that is not
  approved), and generating the JSON with upstream's JavaScript tool was
  rejected as the only path (a Swift author could not add a component without
  running JavaScript).
- **The first version supports the kinds upstream's two real libraries use**
  and refuses anything else with an error: string, number, boolean, any, a
  list of allowed words, array, union, inline object, record, optional,
  nullable, a reference to another component, an action slot, a binding, and
  a named type. A new kind is added by a ticket when a library needs it.
- **The output matches zod 4.6.7**, the version in the reference harness. Zod
  writes "string or number" differently across versions; 396 programs parsed
  against both styles gave no differing result, and only the text sent in
  cloud prompt mode differs. A zod upgrade in the harness is a deliberate
  change with a diff to review.

## Refusals and limits

`createLibrary` refuses what upstream refuses, with upstream's message text: a
root that is not a component, and a function or action name that collides
with a built-in, a reserved call, a component, or another function or action.
A library definition from a host is capped at the session's existing limit for
host input (8 MiB by default, configurable). Upstream has no cap; this is our
addition, as at every other entry point (ADR 0001).

## How it is proved

The reference harness runs upstream's real libraries (react-ui, and Intelligent
UI from a second upstream pin for the library only, since it does not exist at
the lang-core pin and lang-core's source is identical at both commits). A
converter in the harness turns upstream's zod declarations into the plain-data
form; the core's output for that must equal upstream's own JSON Schema and
spec, byte for byte, for every component and function of both libraries. No
library declaration is written by hand in Rust.

## Scope

- Swift gets the full wrapper in OpenUINative. WebAssembly gets the plain-data
  entry point only: a JavaScript author already has zod and upstream's tools.
  Kotlin gets the plain-data entry point through the shared binding, and its
  typed wrapper with the Compose work.
- Upstream's `library.extend` (deriving a library from another) is left out
  and recorded as not yet ported. Nothing planned uses it, and it can be added
  later without changing what this builds.
- OpenUINative's existing built-in components are not moved onto the new
  mechanism here.

## Consequences

`CONFORMANCE.md`'s "Deliberately not ported" entry for the authoring API is
reversed when the work lands, with a pointer to this record. The glossary
gains **Library definition**.

(2026-11-22: since #136, the core's library definition is checked against the complete Intelligent UI package. The react-ui proof is pending support for declarations the converter refuses.)
