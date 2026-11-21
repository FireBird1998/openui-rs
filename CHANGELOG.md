# Changelog

## Unreleased

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
