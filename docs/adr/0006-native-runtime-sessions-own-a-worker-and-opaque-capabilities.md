---
status: accepted
---

# Native runtime sessions own a worker and opaque capabilities

UniFFI objects must be Send + Sync. Action runs, their work accounting and
mutation continuations are thread-affine. Each RuntimeSession owns one worker
thread. Its Store, prepared references, plans and continuations are constructed,
used and destroyed there. A mutex serializes command submission; callers wait for replies after releasing
it. Commands execute in order. Calls from the worker or within a host destructor refuse before queueing;
infallible reentrant disposal is a no-op (ADR 0009). Other threads queue normally,
including during callbacks. A caught
operation panic returns Worker and ends the worker rather than allowing reuse
of possibly partial state. Disposal and final drop join it from any caller
thread. Neither panic containment nor an action failure promises rollback of
earlier effects.

ParseOutput adds a ParsedProgram object containing Arc<ParseResult>. A session
shares that Arc and uses TreeInput::from_shared_result. Existing parse-result
JSON stays unchanged. Streaming retains the core parser's incremental work;
handing each output to the runtime performs no JSON parse or program clone.
The parser's own result construction and requested serialization remain.

Every successful evaluation retains its root unchanged. Action lookup walks a
root-relative JSON path lazily (props, named keys, canonical decimal indices).
There is no up-front indexing or plan-count bookkeeping budget. Runtime
evaluation determines draw flags; tree serialization adds no independent quota
before publishing a generation. maxJsonBytes bounds host input only. Snapshot,
value and event exports likewise add no quota to already admitted data. Root
retention is bounded by the runtime's
result limits. Original step identity reaches its authority checks (ADR 0004).
Tree JSON is exactly TreeResult::to_json. Evaluation/ActionHandle carry only
session identity, non-wrapping generation and path, never old roots/programs.
Successful draws and accepted program replacements release the preceding root
and generation. A refused program leaves it intact.

Continuations have separate monotonic generations and a bounded ordered worker
table. They survive program/draw changes and consume once using current inputs.
Dropping the last native token upgrades a Weak transport and enqueues its ID on
a nonblocking control channel. Only the worker drops the thread-affine run. The
token never retains the transport/session. Disposal joins and releases entries;
a late token destructor simply finds a dead Weak or closed channel.

At the positive maxContinuations bound, only a new suspension evicts the oldest
outcome; when only failures a pause left fill the slots, a host handoff fails at
its mutation step with the pending-plan limit instead. Its native token records EvictedHandle atomically, so no worker-side
tombstone history accumulates. Non-suspending runs/default/legacy events keep
working. Eviction cannot undo effects or cancel external host operations.
pendingContinuations exposes the retained count and a normal dispatch barrier.

This replaces the initial #41 eager plan index and explicit-only continuation
release after independent review demonstrated ordinary draws being refused and
abandoned tokens filling the table. Worker failures are sticky Worker errors;
normal disposal remains Disposed. Expression failures use Evaluation rather
than masquerading as refused Store writes.

PreparedReferenceValues borrows its supply. The worker uses nested lexical
loops: prepare once, service commands, then leave that scope to replace the
supply. This avoids unsafe lifetime extension, leaked references and a
self-referential struct. Replacement is acknowledged after preparation.

RuntimeLimits uses separate ParseLimits for evaluation/reference preparation,
plus host/program/path byte and pending-outcome bounds. It has no maxPlans or
indexing-work bound. The session creates Store::with_limits from its evaluation
limits. Their fixed result-size ceiling caps individual values and total state,
independently of scaled drawing room; admission and partial initialization are
the runtime's rules ([ADR 0005](0005-admit-total-state-incrementally.md)). Refusals
map to RefusedWrite with the configured cap, including per-entry initialization
refusals. No separate state-cap field or duplicate admission logic is needed.

StreamSession retains one current output and bounded source text. Known unchanged
reads/empty pushes/no-op sets share ParsedProgram by Arc; the native records own
their JSON. Actual updates keep actual parser identities, even for equal JSON,
because default container identity affects Store initialization. There is no
history cache and no JSON-derived authority or identity substitution.

Library function callbacks execute synchronously through the session callback
thread, with JSON-only
arguments/results and runtime validation/caching. See ADR 0009 for containment,
reentry and the host's responsibility to return without awaiting another thread
that needs the same session.

Moving the runtime between arbitrary callers under a mutex cannot preserve
thread-local accounting or drop Rc-backed continuations on their owner.
Integer/JSON handles were also rejected: opaque objects express lifetime and
prevent guessing table slots through the safe application interface.
