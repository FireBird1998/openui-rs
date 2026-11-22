# Changelog

## Unreleased

- WebAssembly `free()` can no longer free another object's memory (#211).
  The generated `free` (and `[Symbol.dispose]`) looked up
  `__destroy_into_raw` through the instance, so an own property, a subclass
  field or a prototype set with `Object.setPrototypeOf` could make it
  release another live object's pointer: a use-after-free with aliasing, on
  every class. The guard now runs the generated free on the helper captured
  at load, as #145 did for an interrupted session, and
  `instance.constructor` is the guarded class (it was the generated
  constructor, which reads its arguments' pointers unchecked). A scan in
  `pointer-redirection.test.ts` fails if wasm-bindgen ever generates another
  read through an instance.
- Swift libraries can declare functions and custom actions with
  `defineFunction` and `defineAction` (#139). Pass them to `Core.createLibrary`
  alongside components. A `LiveProgram` registers the Swift implementations
  for complete and streaming programs, and delivers declared actions through
  `onHostEvent`. The session checks arguments, results and registration limits.
- JavaScript tool results (#144, #212). At each JSON position the driver
  looks up and calls `toJSON` at most once, including for Dates. It enumerates
  an object once and reads each property descriptor and value at most once
  per key, including through a Proxy. Across 28 inputs in Bun, the largest
  increases in median peak process memory and projection time were 20% and
  34%, respectively. Free, dispose and detach share a flush bound of
  `maxContinuations + 1` rounds, including nested stops. At a limit of 1,
  the bound drops from 3 rounds to 2.
- No host JSON overflows the caller's thread (#203). The native binding's
  `generate_system_prompt` reads its spec with serde_json on a stack
  segment of its own (`openui_lang::read_host_json`, the reader a library
  schema's text goes through), generates from it and frees it with a
  loop: a spec nested to serde_json's 127-level bound, which overflowed a
  64 KiB thread at about 25 levels unoptimised, now fits one, and every
  accepted and refused spec and every message is as before. The core's
  `generate_prompt_with_limits` and `generate_system_prompt_with_limits`
  copy parts of a spec on a segment sized for its depth, so a defined
  library's prompt options deep in `components` no longer overflow either.
  `DefinedLibrary::cloud_config` now relies on that segment instead of its
  own (#135). Rust API: `read_schema_json` is replaced by `read_host_json`.
  The core's JSON reader (`parse_json_with_limits`, behind definitions,
  prompt options and session state) frees what it read with a loop when it
  refuses text and when a repeated key replaces a value; before, serde_json's
  recursive drop freed it on the caller's stack. TESTING.md lists every
  place host JSON is read and why it fits, and `scripts/check.sh` (so CI)
  now also runs the stack-depth test files at opt-level 0
  (`cargo test --profile dev`), where frames are largest.
- Library definitions support JSON defaults on props and nested fields,
  unknown values and array minima (#209). The reference converter also
  accepts custom refinements on component props and a single string description
  on a string or optional string. The full react-ui definition reproduces
  the pinned schema, spec, prompt and cloud config. A defaulted prop stays
  required in the schema and prints with `?` in the prompt signature.
- The Intelligent UI library as the model sees it (#158). A new library
  product, `OpenUIIntelligentUI`, in the Swift package declares upstream's
  Intelligent UI library (0.0.2) in Swift with `defineComponent`: the 52
  components in their six groups under `Col`, props, descriptions and named
  types as upstream's, and upstream's prompt options
  (`IntelligentUI.promptOptions`: bindings, 19 additional rules, 3
  examples). `Core.intelligentUILibrary()` builds it. Its only additions,
  listed in `IntelligentUIAdditions`, are `Chart`'s `radar` and `radial`
  types and the words describing them. Every component draws a placeholder
  naming it for now. Upstream's 59 functions come with #160, which starts
  right after this: until then the library's prompt lists no `@` functions
  although upstream's rules mention them, and a test fails as soon as
  functions are declared so the exclusion is removed. A Swift test compares the
  library's JSON Schema, prompt spec and prompt with upstream's own, byte for
  byte, once the additions are taken out and upstream's functions left out.
  The Intelligent UI fixture now also records upstream's prompt options and
  the prompt its library writes with them, which the core reproduces byte for
  byte. [Swift](docs/library-definition.md#the-intelligent-ui-library-in-swift)
- The query cache audit accepts a reader waiting on an incomplete query
  statement (#210). On a fresh manager, both the runtime and lang-core
  1.1.0 leave the reader at its default and issue no call. A registered reader
  that becomes incomplete keeps its last wait when its source delivers.
- WebAssembly: an exception escaping a wasm frame no longer changes other
  sessions' output, nor kills the instance by degrees (#208).
  - **Ordinary errors** never pass through a wasm frame and change nothing
    else: errors Rust returns, the guard's refusals, and anything a
    registered function throws or returns. The function bridge no longer
    throws: the guard's wrapper hands Rust a tagged outcome (a leading NUL,
    which JSON never has), the generated import no longer catches, and the
    `@Name threw: …` diagnostics are byte for byte as before.
  - **Behaviour change: any exception that leaves a wasm export ends the
    instance** (a JavaScript stack overflow inside wasm frames, a trap, a
    generated helper failing while Rust waits on a host function). Every
    later call in every session throws `Disposed` ("the WebAssembly instance
    was interrupted by an exception it cannot recover from; load the package
    again"), `free()` succeeds without entering Rust, and wasm-bindgen's
    finalizers do nothing. Before, other sessions went on with wrong output
    (`$a` read as a state reference, every depth limited), and about 100
    stack overflows walked the shadow stack off a leaked stack segment until
    every call trapped. #145's per-session retirement is replaced: the guard
    calls no Rust once it sees an escape. The module exports
    `__stack_pointer`, which the guard puts back.
  - The native binding was already unaffected: a host function's panic
    unwinds and is caught at the callback boundary, and a test now shows it.

- Prompts and bindings from a defined library (#135). A library made from a
  definition gives its system prompt, `prompt(options)`, byte for byte as
  upstream's `library.prompt(options)`, and its OpenUI Cloud config block,
  `cloudConfig(options, instructions)`, byte for byte as upstream's
  `generateSystemPrompt({cloud: true, library: library.toSpec(),
  promptOptions, instructions})`, in the native binding (Swift and Kotlin
  through the generated binding) and the WebAssembly package, through one
  shared policy in `openui-session` and the core's new
  `DefinedLibrary::prompt` and `DefinedLibrary::cloud_config` (each with a
  `_with_limits` form). The options (upstream's prompt options as JSON text)
  and instructions are host input, held to the limits the library was made
  with: `LimitExceeded` past `maxJsonBytes`, `InvalidPromptSpec` for options
  that are not JSON or nest past `evaluation.depth`, `Prompt` with
  upstream's message for a refused prompt; both are absent for a library
  made from a JSON Schema. `conformance/golden/library-definition-prompts.json`,
  written by `reference/library-definitions.ts` from upstream's own
  `library.prompt` and `generateSystemPrompt`, holds 121 prompts (every
  sample, the bindings mode for every sample with a binding, an action slot,
  a function or an action, and every other option on four) and 82 cloud
  configs or errors, all matched byte for byte in the core, the native
  binding and the WebAssembly package.
  New binding methods: `ComponentLibrary.prompt(options)` and
  `ComponentLibrary.cloudConfig(options, instructions)` (Swift
  `prompt(options:)` and `cloudConfig(options:instructions:)`, Kotlin
  `prompt(options)` and `cloudConfig(options, instructions)`, WebAssembly
  `library.prompt(options?)` and `library.cloudConfig(options?,
  instructions?)`), described by the package guard and the binding API
  check. The cloud config's generator copies the library's JSON Schema and
  the options with serde_json, which recurses once per level, so it runs on
  a stack segment sized for the request's depth (6 KiB a level, nearly
  three times the 2,184 bytes measured for nested objects unoptimised):
  definitions at the type depth limit and options as deep as the reader
  allows work on a 64 KiB thread at opt-level 0 too.
  **Behaviour change (bindings):** a runtime session over a defined library
  created with no prompt spec now takes its binding props
  (`DefinedLibrary::binding_props`, the `"binding": true` props) from the
  definition, so an input bound to one writes state as it does when the
  prompt spec supplies the marks (spec #123 story 24). Before, such a
  session had no binding props.
  **Behaviour change (root):** such a session also takes its root from the
  definition, so its streaming parser (raw-response draws) picks the root
  component's statement in a program with no `root` statement. This is
  upstream's: react-lang's `src/hooks/useOpenUIState.ts`, line 108 at the
  pinned commit 47b9cae, creates its parser with
  `createStreamingParser(schema, library.root)`, and lang-core's
  `library.ts` keeps `root: input.root`.
  `conformance/golden/library-definition-roots.json`, written by the same
  script from upstream's own streaming parser, replays it. Before, such a
  session had no root. A prompt spec, when given, still decides the bindings
  and the root, and a library from a JSON Schema is unchanged.
  **Rust API (breaking):** `openui_session::DefinedLibraryOutput` is
  removed: `define_library` now returns the `ComponentLibrary` itself, and
  `openui_session::ComponentLibrary` gains a `defined:
  Option<Arc<Defined>>` field (None for a JSON Schema library; code that
  builds one literally adds `defined: None`), whose `Defined` holds the
  definition, the JSON Schema and prompt spec texts and the limits the
  library was made with.
- WebAssembly session recovery after a throw (#145). A registered function's
  `RangeError` produces the usual evaluation diagnostic. If a throw escapes
  the binding while Rust holds the session, later calls report `Disposed`
  and `free()` succeeds. Module functions now throw `TypeError` when called
  with `new`. The documented `extractMcpToolResult` signature now uses the
  generated parameter name `response_json`.
- Library definitions against upstream Intelligent UI (#136). The reference
  harness converts the library's declarations at its own pin. The core
  reproduces its JSON Schema and prompt spec byte for byte and parses the
  whole corpus identically with the definition and with the upstream schema.

- OpenUINative can define and draw a Swift component library (#138).
  `defineComponent` pairs ordered typed props with a SwiftUI view, and
  `Core.createLibrary` builds the library and its prompt spec. A live program
  draws those views with evaluated props and uses its fallback for viewless
  declarations. [Swift example](docs/library-definition.md)
- A refused query registration reports its error beside a successful draw,
  including when a tool name cannot become text (#187). A re-read keeps held
  data after a refusal only when its registration inputs match, including state
  declarations and their defaults. Neither binding can reach the defensive
  `EvictedHandle` report for a provider-run action today because every run,
  legacy run and resume with an attached provider takes the managed mutation
  path (`query_supply` is `Some`), which refuses new calls at capacity in
  `dispatch_mutation`. Detach and pause remove provider-run slots, and only the
  private `SessionCore::output` can evict one.

- Runtime: an old query result no longer settles a recreated query (#204).
  Through the runtime's Rust API, a staged delivery
  (`QueryManager::stage_tool_call` and friends) that was neither finished
  nor dropped kept the calls it had copied alive. If a query was then
  removed and registered again with the same key, completing its original
  receipt wrote the old result (or error) into the recreated query and
  ended the recreated query's own call, against ADR 0013 rule 3.
  Completion now checks that the entry still holds the receipt's very call,
  as the delivery audit does, instead of only that an entry with the key
  exists. The same check now decides whether a payload is discarded before
  it is read, which run remembered for a key a delivery starts, and
  `QueryManager::abandon_unperformed_call`. Such a receipt is discarded the
  way a released one is: the command succeeds, consumes the receipt and
  changes nothing, and failure text past the limits is not read; abandoning
  it returns false. The session and both bindings finish or drop a staged
  delivery within the completion that made it, so apps on them were not
  affected. No change for any other receipt.
- A component library's size limit, shared action shapes and no unbounded
  recursion (#140). **Behaviour change** for an existing entry point,
  approved by the maintainer: a component library supplied as JSON Schema
  is now host input like a library definition, held to the session's
  `maxJsonBytes` (8 MiB by default), counted in UTF-8 bytes before it is
  read, and refused over it with the existing `LimitExceeded` error ("host
  input byte limit"); one of exactly the limit is accepted. The plain
  constructor (`new ComponentLibrary(schema)`, Swift `init(schema:)`, Kotlin
  `constructor(schema)`) applies the default; the new
  `ComponentLibrary.fromSchemaWithLimits(schema, limits)` (native through
  the generated Swift and Kotlin bindings, and a static in WebAssembly)
  applies the limits given. Malformed text within the limit is
  `InvalidSchema` with the same message as before. One shared policy,
  `openui_session::library_from_schema`, serves both bindings; the package
  guard, the binding API check and the documents cover it, and
  CONFORMANCE.md lists the limit with the port's own.
  The compiled `Library` now holds each action shape once and shares it
  between every param that refers to it, where upstream's
  `inlineActionRefs` copies it into each. Validation reads a param's schema
  as upstream's copy has it, so no parse result, validation error, message,
  order or path changes: a differential run against the previous build over
  the conformance corpus, the library-definition samples and seeded random
  libraries and programs found none. Loading costs time and memory in step
  with the library's size: a chain of 2,000 actions takes about 20-30 MB and
  tens of milliseconds (4.7 GB and 4.6 s before), 30 actions each naming
  the next twice under 1 MB (terabytes before), 2,000 params referring to an
  action with 2,000 params about 10 MB (1.7 GB before), through a definition
  and through a JSON Schema, natively and in WebAssembly, which those shapes
  no longer leave unusable. Compiling is linear in the number of props (the
  `required` lookup is a set). `Library`, `Param`, `LibraryDefinition` (its
  `TypeDescription`) and `DefinedLibrary` copy and free their trees with
  loops, so compiling a schema nested to the reader's limit, parsing with
  it and dropping any of them work on a 64 KiB thread, unoptimised too;
  `DefinedLibrary::library` no longer needs a stack segment of its own.
  Rust API: `Param::schema` is now the param's schema as the library wrote
  it, with `$ref`s to action shapes left in place (it was the expanded
  copy), and `Param` and `TypeDescription` implement `Drop`, so their fields
  can no longer be moved out by a pattern. A Rust caller that needs an
  action's shape follows the `$ref` itself in the JSON Schema it compiled
  the library from (`$defs[<name>]`; for a defined library,
  `DefinedLibrary::to_json_schema()`), since `Library` exposes no shapes,
  and checks a value against one by parsing with the library.
  Reading a schema's text in the bindings runs serde_json on a stack
  segment of its own sized for the reader's 127-level bound (the new
  `openui_lang::read_host_json`), so a schema nested to that limit, or one
  past it, cannot overflow the caller's thread (64 KiB, unoptimised, and in
  WebAssembly); what is accepted or refused, and every message, is as before.

- Session, WebAssembly: response state's Cloud script names are bounded
  before they are read and charged per name (#143, from #126). The
  WebAssembly binding handed the scripts JSON to a JSON reader before the
  session counted its bytes, so a host calling the raw exports
  (unsupported, ADR 0007) could make it parse any text it could put in
  memory; the session now holds that text to `maxJsonBytes` before reading
  it (`LimitExceeded`, "host input byte limit") and counts the names before
  building any (new `SessionCore::script_names`, which the binding calls
  once the session is borrowed). Each script name now costs four bytes
  beside its UTF-8 text, in both bindings and the package guard, instead of
  one: 2.8 Mi empty names, which fit 8 MiB of scripts JSON and passed, are
  refused (`LimitExceeded`, "response state byte limit"), and at most 2 Mi
  are admitted at the default limit. The session also stopped reserving a
  hash-set slot per name before deduplicating them: a draw of 2 Mi empty
  names took about 100 MB of extra heap and now takes a few hundred bytes
  (measured on a 64-bit target). No new limit and no change to the
  generated Swift and Kotlin declarations or the TypeScript types.
  `docs/runtime-binding.md` also says that a native driver redrawing with
  `evaluate()` while its response bundle is blocked reports not blocked.
- Runtime: a query may wait on a statement that is not a query, as
  upstream allows (#188, from #125). When the parser lists such a
  statement in a query's wait list (a Query wrapped in a call named after
  an `Object.prototype` member, such as `@toString(...)`, which the parser,
  like lang-core's, counts as a reserved call), the query
  shows its default and never calls, exactly as lang-core 1.1.0 does; this
  was already the behaviour and is now the decided one (ADR 0013). The
  cache audit's rule 4, and with it the query fuzz target, accept that
  wait; a wait on a query statement still needs a source that explains it.
  No change to any snapshot or call. New pins: the parse case
  `hand/251-a-reader-of-a-wrapped-query` and the reader scenario
  `a-reader-of-a-statement-that-is-not-a-query-keeps-its-default`.
- Runtime, session: three defects on the unregistered and paused paths
  (#142, from #125 and #127). After pause and resume, a program whose own
  re-registration is refused by a limit keeps its results as before and
  now also asks again for the calls the pause released, with the tool
  names and arguments of its last accepted registration (new
  `QueryManager::request_owed_calls`): a reader whose call was in flight
  at the pause shows a call instead of staying idle with no result, error
  or call. While a program is unregistered, a late delivery, a mutation
  result or a pause of the other program no longer sets `changed` or
  `reader_rounds_limited` in `take_query_update`; its calls stay with the
  session as before. Pending runs plus failures waiting from a pause now
  never exceed `maxContinuations`: with only waiting failures in the
  slots, a host handoff fails at its mutation step with the pending-plan
  limit (`size_limited`, no continuation, and the mutation's `run` event
  is not handed to the host), as a managed submission at the limit
  already did; until now it was accepted, reaching `maxContinuations + 1`
  (new `ActionContinuation::step`). A new test bounds the allocation
  requests of each command on the unregistered path to grow in proportion
  to the program.
- Library definitions through both bindings (#137, spec #123). A host hands
  a library definition in plain-data form, as JSON text, to
  `ComponentLibrary.fromDefinition` (native: Swift and Kotlin through the
  generated binding; WebAssembly: a static of `ComponentLibrary`) or
  `fromDefinitionWithLimits`, and gets a component library usable everywhere
  one supplied as JSON Schema is: parsing, streaming, runtime sessions and
  prompts, with identical results. Its `jsonSchema()` and `promptSpec()` give
  the JSON Schema and prompt spec as JSON text, byte for byte as upstream's
  `JSON.stringify(library.toJSONSchema())` and `toSpec()` write them (names
  that look like array indices first); both are absent for a library loaded
  from a JSON Schema. The definition is host input: it is held to the
  session's `maxJsonBytes` (8 MiB by default), counted in UTF-8 bytes before
  it is read, and refused over it with `LimitExceeded` ("host input byte
  limit"); its types nest at most `evaluation.depth` levels. Malformed JSON,
  a definition not in the form, an unsupported kind, a root that is not a
  component and a colliding function or action name are refused with
  `InvalidJson` and the core's message (upstream's for the root and the
  collisions). One shared policy, `openui_session::define_library`, serves
  both bindings. The package guard describes the new members, both packages
  load a defined library, and the binding API check covers the entry points
  in both bindings and in the generated Swift and Kotlin declarations. New
  page `docs/library-definition.md` documents the plain-data form, every
  kind with an example, what is refused or can't be written, and what the
  byte limit does not bound: the parser's library copies each action's shape
  into every param that refers to it, as upstream does, so a chain of 2,000
  actions (194 KB) takes 4.7 GB, a doubling chain of 18 (2.6 KB) 2 GB, and
  freeing such a library overflows a 64 KiB thread past about 150 chained
  actions (#140).

- OpenUINative: typed errors and a typed cloud prompt (#148, spec #129).
  Every public call that can fail now throws only `OpenUIError`
  (`throws(OpenUIError)`), a `kind` a caller can switch on and a
  `message`. The binding's refusals keep its kind, one to one, so an app
  no longer needs the generated `OpenUiError`, which it cannot import;
  OpenUINative adds kinds for its own refusals (`stale`, `streaming`,
  `blocked`, `unusableElement`, `toolProviderAttached`, `invalidArgument`,
  `decode`, `cancelled`, `unexpected`). New typed prompt calls:
  `Core.systemPrompt(library:options:)` gives the full system prompt and
  `Core.cloudConfigBlock(library:options:instructions:)` OpenUI Cloud's
  config block, from `PromptOptions` (with `PromptTool`, `ToolSpec` and
  `ToolAnnotations`) and `CloudPromptOptions`, which holds only examples,
  a preamble and additional rules, as upstream's `CloudPromptOptions` does.
  Both equal upstream's `generateSystemPrompt` for every recorded case the
  typed options can say, including a new recorded case,
  `conformance/prompts/react-ui-cloud.json`: react-ui's library and prompt
  options in cloud mode, as upstream's openui-cloud template asks for it.
  The core already made the block; nothing in Rust changed. Breaking Swift
  changes, and how callers migrate: `LiveError` is gone, so match
  `OpenUIError` and its `kind` instead (`#expect(throws: LiveError.self)`
  becomes `#expect(throws: OpenUIError.self)`); `DecodeError` is gone from
  OpenUINative (the viewer keeps its own); a superseded
  `LiveProgram.show(response:streaming:)` throws `OpenUIError` of kind
  `.cancelled` instead of `CancellationError`; catching the binding's
  `OpenUiError` around an OpenUINative call no longer matches, so match the
  kind (`catch OpenUiError.LimitExceeded` becomes
  `catch let error where error.kind == .limitExceeded`). A refusal's text
  in `failure` is now the binding's wording for every kind (`session
  disposed`, `invalid JSON Schema: …`) where a few kinds used to show
  Swift's description of the case. `JSONValue.compact` and `==` now walk
  an explicit stack instead of recursing, with the same text and results:
  a 511-level schema that `JSONValue.parse` accepts, handed to the typed
  prompt calls or returned by a tool provider, crashed a debug build on the
  cooperative pool (stack overflow from 440 levels); it is now refused by
  the core as the raw spec is. `==` keeps `Array ==`'s answer for two
  arrays sharing one buffer, so an array holding NaN still equals itself.
  `PropValue.summary` and `holdsElements` walk an explicit stack too, with
  the same output: summarising a list a model nested to the core's depth
  limit (255 levels) crashed a debug build off the main actor.
- Library definitions: functions and actions (#134, spec #123). A library
  definition can declare `functions` (a name, a description, ordered
  `params` and an optional `returns` type) and `actions` (a name, a
  description and `params`), as upstream's `defineFunction` and
  `defineAction` do; a function's implementation never enters the core and
  still reaches the runtime through the host-function path. Their
  signatures are built by the core (` → T` after a function's when it
  declares a return type with an annotation) and written into the prompt
  spec's `functions` and `actions`, and the JSON Schema's after `$defs`,
  byte for byte as upstream's lang-core with zod 4.6.7 writes them, quirks
  included: a call's `$ref`s refer to the library's `$defs`, which have an
  entry only when a component refers to it too. A component's action slot
  can name a library action (`{"kind": "action", "name": "Copy"}`), printed
  `@Copy`, whose `$defs` entry `{type: const name, params}` appears only when
  a component refers to it. `create_library` refuses a function or action
  name that is a built-in, a reserved call (including the names every
  JavaScript object inherits, as upstream's `in` check sees them), a
  component's, or an earlier function's or action's with the new
  `LibraryDefinitionError::NameCollision`, whose message is upstream's.
  Refused as invalid because the output can't be copied: a library action
  named `ActionExpression` (upstream registers it under the action slot's
  id, and zod throws when a library refers to both) and a reference to the
  library action with the empty name (upstream writes it inline). A param
  has no `binding`. New public `FunctionDefinition` and `ActionDefinition`;
  `LibraryDefinition` gains `functions` and `actions` (a breaking change for
  code that builds it literally, #146); the `UnknownAction` message now
  mentions the library's own actions. The reference converter carries
  functions and actions, finds a library action's ref by identity, and
  refuses by name what the plain data can't say (a binding on a param or a
  return type, an optional return type, a params object shared with a
  component, an action outside the library, the two cases above), and a
  declaration in which zod writes a shared schema as a `__schemaN` `$defs`
  entry: the plain data has no object identity. The converter replays zod's
  walk for each conversion upstream makes and refuses exactly those whose
  output has such an entry or a call's `$ref` to one (no over- or
  under-refusal in 18,000 random libraries with shared params objects). 38
  new samples (93 in all) match upstream byte for byte, and 95 converter
  refusals are checked, the `__schemaN` ones against upstream's output.
- Admitting a tool result given as a value no longer takes time exponential
  in its depth (#141). A value that shared its child twice at every level
  used to be read at every position it fills to decide whether it is JSON
  and within the depth limit. A shared body is now remembered with how much
  room it was read with; one read whole is not read again, and one cut at
  the depth limit is read again only where it has more room, so the depth
  limit multiplies the work only for a value that is refused anyway.
  Nothing past the depth limit is read, and the size check still stops at
  the first byte over the limit. Every value is decided in the same order
  with the same outcome as before, at any depth limit up to `usize::MAX`:
  validity, then depth, then size. A reader or source id cut for
  the cut-source error no longer exceeds its room when the room is under
  three bytes; it is empty instead of a bare ellipsis. Rust: the hidden
  `ToolOutcome::ToolError` constructor is gone; use
  `ToolOutcome::CallFailed`.
- Library definitions: the kinds that link a library together (#133, spec
  #123). A prop's type can refer to another component of the library
  (`{"kind": "component", "name": "Item"}`), be an action slot
  (`{"kind": "action"}`, or `{"kind": "action", "name": "OpenUrl"}` for one
  built-in action, so a slot restricted to several is a union of them), or
  be a named type (`{"kind": "named", "name": "Color", "type": T}`); a prop
  can be marked a binding (`"binding": true`, on the prop, so it marks only
  the outermost type). Each is written as upstream's lang-core with zod
  4.6.7 writes it: a reference as a `$ref` and the component's name, an
  action slot as a `$ref` to `ActionExpression`, `OpenUrl` or `ToAssistant`
  (printed `ActionExpression` or `@OpenUrl`), whose `$defs` entry appears
  only when a component refers to it and not at all when a component has
  its name (the action is then written inline), a binding as
  `$binding<T>` with the unmarked type's schema, and a named type as its
  name in the signature and its structure in the schema, with upstream's
  quirks (a named nullable prints its inner type, an empty name is none, an
  array of the action slot or of a named union is parenthesised). `$defs`
  come in upstream's order, verified by running upstream on libraries whose
  components refer to each other forwards, backwards, cyclically and to
  themselves: the order zod first meets each schema walking the components
  depth first. The walk visits each component once with its own stack, so a
  cycle or a reference chain as long as the library costs nothing more
  (2,000 components in a cycle on a 64 KiB thread). A reference to a name
  that is no component's is refused with the new
  `LibraryDefinitionError::UnknownComponent`, an action name a slot can't
  name with `UnknownAction`, and a reference to the component with the
  empty name, or a named type over a reference, a slot or another named
  type, as invalid. `TypeDescription` gains `Component`, `Action` and
  `Named`; `PropDefinition` gains `binding` (a breaking change for code that
  builds it literally, #146). The reference converter now carries component
  references (found by identity), action slots, named types (a component
  outside the library is one) and bindings on a prop's outermost type, and
  refuses by name a binding anywhere else (upstream prints it but never
  binds it), a prompt name on `.optional()`, a reference to the empty name
  or to a component a later one of the same name shadows, and a custom
  action's ref (#134); 16 new samples pin each case.

- Library definitions: the remaining data kinds (#132, spec #123). A
  prop's type can also be a list of allowed words (`enum`), an `array`, a
  `union`, an inline `object` with ordered fields, a `record` or `nullable`,
  nested in one another; optional and nullable are separate facts, and the
  order of fields and union members is kept. Each is written as upstream's
  lang-core with zod 4.6.7 writes it, in the JSON Schema (an `anyOf` of bare
  types folded into one `type` list) and in the signature (a nullable prop
  marked `?`), with upstream's quirks: enum words in JavaScript key order,
  each once; an empty enum or union as upstream writes it. A type nests at
  most `Limits::depth` levels: `LibraryDefinition::from_json_with_limits`
  takes the limits, and a deeper type is refused with the new
  `LibraryDefinitionError::DepthLimited` (200,000 levels are refused at the
  limit on a 128 KiB thread). `DefinedLibrary::library()` compiles the schema
  on a stack segment sized for the definition's depth, so at the default
  limit reading, `library()`, `to_json_schema()` and `to_spec()` work on a
  64 KiB thread. What they return is dropped by the caller through
  serde_json's recursive drop: at the limit that fits 64 KiB in optimized
  builds but not at opt-level 0 (free the JSON with `free_json`; the
  returned `Library` has no such way out). New public
  type `FieldDefinition`; `TypeDescription` gains six variants. The reference
  converter carries the new kinds and refuses by name what the plain data
  can't say (literals, tuples, discriminated unions, intersections, records
  with other keys, `.optional()` inside a type, component references and
  action slots, which are #133's, symbol keys in an object's shape, types
  nested past the core's depth limit including a recursive getter schema,
  and more); 16 new samples pin each kind.

- Library definitions in the core (#131, the first ticket of spec #123,
  ADR 0017): a host describes a component library as plain data, components
  with a name, a description and ordered props whose types are string,
  number, boolean or any, optional or not, plus component groups, a root and
  an id. `LibraryDefinition::from_json` reads it, `create_library` makes a
  `DefinedLibrary`, and its `to_json_schema()` and `to_spec()` give what
  upstream's `toJSONSchema()` and `toSpec()` give for the equivalent zod
  declaration (zod 4.6.7), byte for byte, signatures included. A root that
  is not a component is refused with upstream's message; a type kind the core
  doesn't support yet is refused naming the kind. `DefinedLibrary::library()`
  compiles the JSON Schema, so the parser is unchanged and a defined library
  parses as its JSON does. The reference harness gains a converter from zod
  declarations to the plain-data form (`reference/library-definition.ts`),
  which refuses by name whatever the plain data can't carry (a zod format,
  check or catch-all, metadata, a props object shared by two components, a
  component group with other fields or another field order), and sample
  libraries whose upstream output is generated
  (`reference/library-definitions.ts`); the corpus gains three programs for
  the sample library. The binding entry points come with #137.

- Breaking binding and Rust API changes, accepted because nothing has been
  released; the old names are not kept as aliases (#120):
  - The MCP result helper's outcome for an error the tool reported is
    `McpToolOutcome::ToolReported` in Rust, formerly `ToolError`, which read
    like the query manager's `tool-error` (a call that failed). The helper's
    JSON is unchanged: `{"error": …}` is still the `completeToolCall`
    outcome Swift's `ToolProviderError.toolReported` maps to.
  - A message attribute name or value that cannot be URI-encoded (a lone
    surrogate, where upstream throws a `URIError`) makes `buildMessage`
    throw the new error kind `InvalidAttribute` in both bindings, formerly
    `InvalidJson`. Its reason text is unchanged, and malformed parts are
    still `InvalidJson`.
  - WebAssembly exports `extractMcpToolResult(response_json, limits)` as a
    free function, formerly the static `RuntimeSession.extractMcpToolResult`,
    so it has the native binding's shape and the other stateless helpers'.
    The package guard wraps it as it wraps them.
  - Rust: `openui_lang::normalize_js_string` is gone; the core's one
    surrogate-pair normaliser is `openui_lang::normalize_surrogate_pairs`,
    which takes borrowed or owned text and returns it uncopied when it holds
    no pair. A differential test showed the two former normalisers gave the
    same result on every input before they were merged.

- An action whose mutation is dropped because queries are paused is reported
  once as an action failure with the new error kind `QueriesPaused` in both
  bindings (#117), where it used to vanish with no call, error or event. A
  mutation step clicked while paused fails in the click's own output; a plan
  whose call was in flight at the pause comes back from the next query update
  as a segment with that failure. Nothing more of the plan runs, and the late
  outcome is still ignored. The JavaScript driver now also reports each
  resumed segment's failure to `onError`. Freeing or disposing the session
  delivers every segment the session produced, a pause's failures and the
  rest of a drain the free was made inside and segments its callbacks
  produce included (up to three takes, then one `LimitExceeded`), and after
  it returns no driver of the session, current, closing or detached earlier,
  calls a callback, invokes a provider or delivers a result (ADR 0014).
  Detach delivers the same way and reports the evaluation error it takes. The reason says whether the call was handed
  to the tool provider. A host handoff counts waiting failures against the
  slots.
  OpenUINative shows the failure in `LiveProgram.failure`. A query `@Run`
  that cannot fetch, paused or unregistered, is skipped and the plan carries
  on, as upstream's is (ADR 0013).

- Every draw reports the renderer's `blocked` for its response:
  `EvaluationOutput.blocked` in both bindings (#118). It is true while the
  response streams and, for a response bundle, while it is incomplete or has
  an invalid scripts section, as upstream passes `isStreaming: blocked` to
  its components. OpenUINative's `LiveProgram` reads it, gains `isBlocked`,
  and draws buttons and inputs disabled while it is true; the JavaScript
  driver returns it with each `setResponse` draw. The session's click and
  Retry gate is unchanged. The WebAssembly package counts response state in
  UTF-8 bytes instead of UTF-16 units and refuses scripts JSON over
  `maxJsonBytes` bytes, which escapes could make about six times the names
  before (#108).

- Streaming a raw response costs about what pushing it does, in both
  drivers: `RuntimeSession.setResponse` and `LiveProgram.show(response:
  streaming:)` read only what an append adds and parse only the new part of
  the program, through `openui_lang::ResponseReader` (ADR 0015). Results are
  unchanged. 64 KB in 16-byte chunks through `setResponse` took 2.0 s in
  release WebAssembly in Bun, against 5.4 s before and 2.5 s for push with
  `replaceProgram`. The native `StreamSession` gains `readResponse`, which
  OpenUINative's `Session` uses with its own parser, and
  `StreamParser::try_set` stages an update with journals instead of copying
  the parser (#116).

- The WebAssembly tool provider driver yields between dispatch rounds through
  `setImmediate` or a `MessageChannel` instead of a zero-delay timer: 200
  dependent calls take about 7 ms in Bun instead of about 270 ms, and a few
  milliseconds instead of about 0.9 s in Chromium. A tool result holding a
  `Map`, `Set`, nested or boxed symbol, boxed non-finite number, invalid
  `Date`, typed array, `ArrayBuffer` or `DataView` now ends the call with a
  readable `tool-error` instead of passing through `JSON.stringify` silently.
  The native binding gains `finishToolCall`, a terminal completion that
  settles a refused outcome as the call's tool error; OpenUINative's driver
  uses it, so an oversized failure message no longer leaves a query loading
  (#109).

- CONFORMANCE.md, the README, TESTING.md and the runtime crate's overview
  describe the ported query manager, the tool provider and the session
  operations. A `query` coverage-guided fuzz target replays the query manager
  scenario format, seeded from the committed scenarios; the nightly hunt and
  the GitHub fuzz workflow run it. `scripts/mutants.sh query` runs the
  mutation group for the query manager (#95).

- OpenUINative's `LiveProgram` accepts an async tool provider through
  `attachToolProvider`. Apps can read query status, retry failures, pause or
  resume queries, and detach the provider. The last `ElementView(live:)`
  disappearing pauses queries. Stream previews and the app's streaming
  request defer new query registrations (#92).

- With a tool provider, an action's `@Run` of a query runs it again through
  the query manager and emits no host event; without one the `run` event is
  unchanged. Sessions and both bindings gain `retryFailedQueries`. Rust adds
  `QueryCommand::Invalidate`, `TreeInput::with_query_runs` and
  `ActionRun::query_runs` (#87).

- Rust query-manager pattern matches on invocation failures now use
  `ToolOutcome::CallFailed(message)`. The former `ToolOutcome::ToolError(message)`
  spelling stayed as a construction helper until #141 removed it. MCP
  tool-reported errors keep their distinct meaning (#84).

- Binding session construction now refuses an explicitly configured
  `referencePreparation.peakMemory` with the typed `LimitExceeded` error (#74).
  It was previously silently ignored. Configure evaluation memory on
  `evaluation.peakMemory` instead; leave reference preparation unset (#76).

- Breaking Rust API change from #49's supplied-input accounting work:
  `openui_lang::PreparedInput` is no longer exported. This is not a rename.
  `PreparedReferenceInput` is the core-issued reference-preparation receipt;
  `SuppliedInput` describes one current program, starting action/message/form
  contribution, and reference receipt. Use the corresponding `PreparedProgram`
  and `PreparedStart` receipts for measured program and starting input.
  Callers cannot supply an arbitrary reference byte claim or combine arbitrary
  receipts. No compatibility alias is provided because the meanings differ.
- `@Each` substitution uses one traversal for typed syntax and descriptor data,
  retaining existing output, diagnostics, work and result-room prices (#49).
  Same-session `with_scope` re-entry retains the suspended owner's accounting;
  its existing nested evaluation charges and sticky cut flags are pinned (#64).
