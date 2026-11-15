---
status: accepted
---

# Custom action identity stays in the runtime session

Evaluating a call to a declared custom action registers the identity of its
shared step object in its store's private registry. This includes program text
and AST-shaped calls supplied as data: an evaluated element prop from state
or query data can contain `{k:"Comp",name:"Go",…}` and invoke a declared `Go`
action with arguments chosen by that data, just as upstream does. Such a call
earns a real mark; a finished custom step supplied as data cannot forge one.
`evaluating_declared_action_calls_in_state_and_query_data_mints_real_steps` pins
both the data invocation and later registry acceptance of its result. The registry holds weak references and verifies
the live allocation, not its address alone. Retained trees, plans and state
hold the strong references. Cloning a Value shares that identity; reconstructing
it from JSON creates a different object and grants no authority. Dead entries
are collected with amortized sweeps. The registry has no public marking primitive; evaluation mints marks for
declared calls.

The mark belongs to a runtime session (currently the Store), rather than a
process-global table. Separate stores cannot authorize each other's custom
steps, unlike upstream's process-wide WeakSet. This boundary is invisible in
JSON and is pinned by `custom_identity_survives_tree_state_and_binding_but_not_json_or_another_store`.
This makes disposal, concurrent sessions and embedding predictable.
Dropping an evaluated tree does not revoke steps still retained in state.
`Store::dispose` ends that session's authority, even for values a caller still
retains and even when state is empty. Reusing the store starts new authority;
it cannot revive old marks. The retained-plan/cut-data test pins this rule.
Future binding disposal must also release all handles and advance their
session generation, so a stale handle cannot regain authority.
Plans on props are filtered like upstream's ownSteps; raw state and snapshots
keep their original values. A plan written through a binding retains its step
identities and can later be run in the same session.
Serializing and restoring that state loses the custom identities.

Swift, Kotlin and WebAssembly sessions will retain the original Rust plan and
return an opaque, session-owned handle alongside their tree/path interface.
Running takes that handle and retrieves the retained Value. A JSON round trip
alone is data and cannot restore custom authority. Handles must reject a wrong
session or a released slot (including stale generations); their concrete ABI
belongs to the binding tickets. These handles are separate from the upstream
JSON representation: tree JSON remains byte-identical, with no secret fields,
IDs or serialized marks. Rust callers likewise hand back retained Values to
the same session. `run_action` now accepts a retained `Value` by reference. The handle tables
remain the binding tickets' responsibility.

A JSON field, signature of JSON contents, or content hash cannot distinguish
a real step from byte-identical supplied data. A process-global registry also
makes lifetime and cross-session authority surprising. Weak allocation identity
preserves upstream's relevant object-identity behavior while allowing bindings
to retain capabilities without changing the JSON format.

A flagged cut tree can contain a raw, forged custom step. Partial-result
recovery preserves raw props and cannot scan arbitrarily large retained input
after work is exhausted. The retained-plan/cut-data test pins both the visible
raw cut and its rejection when evaluated again without a cut. Authority is
always decided by the registry, never by a step's position in a tree, whether
that tree is flagged or complete. The runner must recheck every custom step.

Mutation run steps suspend after returning the host handoff, including evaluated
args. `ActionRun::continuation` is an opaque, single-use capability for that run;
its clones share consumption. It has no JSON representation and retains no
borrow of a tree, program, library or Store. The Store owns its session identity,
revoked by disposal even when empty. `resume_action` checks the identity and
consumption before any effects. A foreign/disposed/consumed token is a typed
refusal. False consumes the token and ends the run. True runs later steps using
the then-current caller-supplied program and Store. A second mutation returns
a new continuation; a last mutation still requires its outcome.
The run's original work/input accounting persists across every suspension;
resuming never resets spent work. Newly supplied input earns bounded additional
credit under ADR 0003. Defaults are initialized lazily at the first
expression and remain frozen for that run. Bindings retain continuations in
opaque session-owned handle slots and release those slots on disposal. The
current evaluator/continuation is thread-affine; native bindings can dispatch
session work to its owning thread. The opaque representation leaves a future
sendable accounting implementation possible without changing this interface.


A retained `Arc<ParseResult>` can be supplied with `TreeInput::from_shared_result`.
The runner holds the current Arc while suspended and caches its measurement and
statement indexes by identity. Thus `Arc::make_mut` by the embedder clones the
program; the old allocation remains immutable. Only the current cached program
is retained. Overriding the public `result` field invalidates the cache. Borrowed
programs, fresh equal Arcs and alternating Arcs are remeasured as necessary,
entirely outside evaluation work. Raw reference input is prepared once per call;
prepared reference input takes precedence without walking unused raw input.
Neither reference values nor prior program measurements/owners are accumulated
across suspensions. The retained plan, lazy computed defaults and current cache
bound suspended run state independently of the number of resumes. Bindings may
retain shared programs to avoid setup work; this optimization changes no effect,
credit or cut point. ADR 0003 defines the one combined supplied-input floor.

Disposal revokes authority immediately. An externally retained revoked continuation
can still retain its private run state until its last clone is dropped. Releasing
all binding handle slots on disposal releases that memory. Keeping the run state
in the thread-affine opaque token avoids making the Store thread-affine merely
for reclamation. No revoked token can evaluate or regain authority.
