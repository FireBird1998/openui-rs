# Changelog

## Unreleased

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
  - WebAssembly exports `extractMcpToolResult(responseJson, limits)` as a
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
  spelling remains a construction helper. MCP tool-reported errors keep their
  distinct meaning (#84).

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
