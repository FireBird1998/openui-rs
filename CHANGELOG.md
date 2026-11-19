# Changelog

## Unreleased

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
