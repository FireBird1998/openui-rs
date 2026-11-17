---
status: accepted
---

# Replace a session program in one atomic operation

A refused Swift show used to restore its drawing after setProgram had already
changed the live Rust program/store. Later app writes could draw that refused
program. One shared-session `replace_program` operation now stages Store state
and private provenance, initializes, evaluates and serializes, and commits only
on success. Values and the action-session identity stay shared; marks are copied
into independent registries, so evaluation of a refused candidate cannot alter
live authority or its amortized sweep bookkeeping. Continuations and callback
registrations are not copied, replayed or recreated. The helper does not wrap
host invocation in stack growth; native callback transport remains ADR 0009's.

A host-managed prepare/commit protocol was rejected: callers could abandon a
prepared transaction, retaining extra state/authority and making ordering and
cleanup part of every binding. A JSON snapshot restore was rejected because it
loses pristine/undefined state, object identity and custom authority. Rollback by
replaying initialization/evaluation was rejected because it repeats host effects
and work. A fresh session per show loses continuations and registrations.

The new call returns all display bytes, snapshot and version. It has an explicit
native-view admission option matching Swift's existing root shape and depth
512 decoder. This catches host conversion refusals before commit without putting
Swift callbacks or a general host transaction protocol into the session. Ordinary
binding callers opt out. Initialization's existing per-entry refusals and runtime
cut flags still commit; they are successful results, not thrown refusals.
Existing separate operations keep their exact contracts. External synchronous
host-function effects cannot be undone, as before.

Store::initialize_transaction performs the two initialization batches internally
and exposes only a shared borrow of the staged store to the evaluation closure.
A mutable borrow was rejected: a caller could move the candidate out on refusal
and create two independent stores sharing session authority. A compile-fail
doctest enforces the shared-borrow boundary.
No candidate or mark crosses JSON. Failed operations retain neither staging
state nor a transaction handle. The cost is a shallow Store snapshot owner plus
copies of bounded name/size/pristine/provenance tables; candidate and old state
coexist until commit/drop. This is not a new peak-heap guarantee.
