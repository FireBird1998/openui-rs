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
identities and can later be run in the same session, when the runner exists.
Serializing and restoring that state loses the custom identities.

Swift, Kotlin and WebAssembly sessions will retain the original Rust plan and
return an opaque, session-owned handle alongside their tree/path interface.
Running takes that handle and retrieves the retained Value. A JSON round trip
alone is data and cannot restore custom authority. Handles must reject a wrong
session or a released slot (including stale generations); their concrete ABI
belongs to the binding tickets. These handles are separate from the upstream
JSON representation: tree JSON remains byte-identical, with no secret fields,
IDs or serialized marks. Rust callers likewise hand back retained Values to
the same session. The runner and handle tables are not implemented here.

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
