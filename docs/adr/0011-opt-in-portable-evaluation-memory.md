---
status: accepted
---

# Make evaluation memory an opt-in portable reservation allowance

The maintainer's final decision on #55 is an explicit memory limit, disabled by
default. Result room cannot bound eager temporaries without cutting programs
that currently finish (ADR 0008). `Limits::with_peak_memory(bytes)` opts in;
`peak_memory()` is `None` by default. Work, serialized room and input credit are
separate and retain their existing charges. The allowance never scales with
input. Parsing syntax and materialisation retain their existing limits and cost.

## Measure

Use portable **reservation bytes**, with the lifetime of a conservative arena.
A successful reservation stays in the account until that evaluation allowance
ends, even if the corresponding intermediate dies. The accounted peak is the
sum of successful reservations. Refused reservations add nothing and latch a
memory cut. Checked addition refuses overflow, including at `usize::MAX`.
There is no allocator instrumentation, `size_of`, identity ledger or GC scan in
this measure. The existing native-capacity limit for transformed arrays remains
a separate, unchanged limit; it supplies no memory-accounting credit.

Setup receipts (ADR 0010) continue to require equal measuring limits. The new
peak-memory setting is excluded from that comparison: preparation neither
measures nor spends evaluation storage. Bindings reject this setting on reference preparation with their existing
`LimitExceeded` configuration error; no evaluation occurs there. Changing the
evaluation setting cannot invalidate
otherwise identical program/start receipts or give additional input credit.
`memory_settings_do_not_invalidate_setup_only_receipts` pins reuse between unset,
profiling and adjacent memory settings, with zero setup memory reservations.

The reservation events and their weights are:

| Event | Portable reservation bytes |
| --- | ---: |
| Container with `n` logical slots | `64 + 128*n` |
| Object | Container slots plus UTF-8 bytes of each key; public-object and own-fields body-copy admissions also include directly owned string text |
| Text copied/read as an evaluated value | `64 + UTF-8 length` |
| Scalar evaluation/projection or other shallow value | 128 |
| Element rebuild | Six envelope slots, type-name text, props object/directly owned string copies and optional statement-id text |
| Admitted serialized output | Existing room-reservation byte count, in addition to structural reservations |

Logical slots use lengths, never a platform's `Vec` growth or native layout.
Array/object evaluation, descriptor array/object evaluation, Each result vectors,
substitution visits/list scratch, literal construction, JS/public projection,
state/default value conversion, supplied-value read boundaries, argument fields,
member plucking, sorting/filtering/elementwise arrays, action-event envelopes,
run-step parameter maps, form payload containers and tree rebuilds reserve
before their known allocations. Sorting reserves its two partitions and each
merge destination. View iterators of unknown length reserve per retained entry.
Own-fields conversion of array-, element- and AST-valued shaped props preflights
immediate owned copies before the runtime helper; lazy descendants remain lazy.
String concatenation/join growth also reserves its existing output-byte charges;
string coercion reserves a 32-byte scalar envelope or the known text length;
UTF-16 scratch reserves twice its UTF-8 length. Eager plain-data conversion of
syntax preflights its fields/children before constructing that representation.

These are deliberately conservative **admission events**, not distinct physical
allocations. Shared reads and scalar visits reserve again. Several events can
cover the same physical bytes; a transferred string still reserves a potential
copy. Room refunds, cache collection, object identity and dropping a result never
refund this account. Thus the measure does not claim to equal instantaneous live
value sizes. In a long sequential run the accounted peak can exceed the physical
peak by an arbitrarily large factor. This is the cost of a simple deterministic
allowance that remains monotonic across suspension, without a new lifetime/GC
system in the one evaluator. A future live-value allowance would be a separately
versioned accounting contract, not a silent discount to these weights.

## Lifetime and cut behavior

Expression sessions and trees have one account, including callbacks and rebuilds.
An action run retains one account across steps and mutation continuations, as it
retains work in ADR 0003. Releasing per-operation output room does not release
memory reservations. Each parse's evaluated defaults share an account; opt-in
streaming evaluates defaults afresh because historical reuse receipts contain
only work/room charges. Unset streaming retains the original reuse path.

A refusal sets the existing `size_limited` flag. Changing every generated result
record to add a reason would exceed the permitted optional configuration change
in the bindings, so the generated result interfaces keep their current shape.
Rust additionally exposes the distinct `EvaluationMemory { accounted_peak,
limited }` diagnostic through `EvaluationSession::memory`, callback evaluators,
`evaluate_tree_with_memory` and `ActionRun::memory`. Disabled accounting reports
`None`; action setup refusal before execution also reports `None`.

A memory-cut tree replaces rejected raw recovery props/root with null: recovery
must not copy a payload whose reservation was refused. Ordinary work/room/depth
recovery is unchanged. A cut binding/form/action write never reaches Store
mutation. Earlier completed steps and host events remain, as in ADR 0003. This is
per-operation refusal, not whole-run rollback; host callback side effects are
outside the Store's transaction boundary.

## What is outside the measure

There is **no process-heap or RSS guarantee**. In particular, the account excludes:

- Program text, parser/tokenizer/materialisation structures, and unevaluated or
  plain defaults retained by parsing. Only evaluated-default work is covered.
- Caller-owned input retained before the allowance starts: parsed programs,
  supplied references, Store state/snapshots, action plans, schemas and registries.
  A read/rebuild reserves its admitted representation; it does not retroactively
  charge the original input allocation.
- Scope/host function callback computation and allocation before the owned-value
  return boundary, including scalar copies made by builtin Store reads. The core
  can admit a returned value before its own conversion/projection, but cannot
  preflight an already completed callback allocation through this interface.
  Also excluded are
  host-owned/retained output, JSON transport and binding-worker infrastructure.
- Evaluation bookkeeping: name/reference/measurement/identity caches, weak/strong
  owner tables, traversal frames, key indexes and diagnostics' bookkeeping. Output
  diagnostic bytes still pass ordinary output reservation.
- Runtime setup/reference preparation, Store admission/copy-on-write and retained
  authority records, other deferred plan/envelope helpers, form collection and validation
  scratch outside the evaluator's reservation paths, and result serialization.
- Rust/allocator layout, capacity growth beyond logical slots, allocator metadata,
  fragmentation, stack/grown-stack mappings and thread-local infrastructure.

These paths have their existing input/work/room/depth bounds where applicable;
their native heap bytes cannot be inferred from this portable measure. No guessed
allocator multiplier makes them counted. The accounting scope therefore bounds
admitted evaluation storage, not every allocation performed by a runtime API.

## Host rule and evidence

For an identical program/input/callback sequence and other limits, profile with
`with_peak_memory(usize::MAX)` and use at least the uncut reported accounted peak.
That value and every larger value admit the same sequence. A lower allowance cuts
at its first refused reservation; it cannot change an already uncut run into a
cut run when raised. For parse defaults, which retain the established parse-result
interface, a host can bisect the smallest uncut allowance on an otherwise uncut
fixed input, or use a conservative application ceiling. Separate evaluations
start separate accounts. A run's requirements include all of its resumed segments.

Public `peak_memory` tests enforce adjacent thresholds, sweeps, descriptor and
reentrant paths, default evaluation, rebuilds, resumed writes and unchanged Store
state. The 2,652-row read/unread repeated-reference witnesses compare set/unset
allocator peaks in the same process, with wide layout tolerance. Varied leaves
record heap/account ratios and enforce `heap <= account` for those finite
shapes, including the 1,000-row repeated-reference profile. This is a regression guard, not a universal multiplier. TESTING.md
records measurements and the default-path controls; no timing is an assertion.

## Alternatives

A default bound is rejected by the maintainer: it would change completed upstream
behavior. Production allocator readings are nonportable and cannot enforce before
allocation. Counting serialized room alone misses temporaries. A weak-owner live
ledger would depend on native ownership/reclamation and need to cover every value
representation and suspended root; it also adds tracking allocations. Keeping
reservations until allowance end provides an explicit, auditable first contract
without changing default allocation counts or reported work/room.

## Review follow-up (#74)

Recovery admissions apply at element envelopes, owned Each rebuilds and later
projection/rebuild boundaries too. A refused envelope returns null before
copying its raw props; completed children remain where already admitted. The
recovery sweep visits both ends of every reservation-equivalent allowance
interval for array, object and nested-element props, with completed JSON plus
32 bytes as the finite-shape bound. An Each test derives a refusal edge from a
one-item control and compares heap with the completed same-process control;
delaying the result-vector admission until after construction fails that bound.

The weights remain unchanged. On the review machine, the 1,000-row
`Each(L,i,L)` profile accounted about 900.7 MB for 48.8 MB of measured heap
(18.46 times). The Each result-vector event itself is only 128,064 reservation
bytes, under 0.02% of the total. Discounting that event cannot resolve the gap;
most charges are repeated reads/projections retained by the arena contract.
Changing their lifetime or globally discounting scalar visits would need a
separate accounting design and broader evidence than these finite shapes.
The measured values are observations, never platform-specific test pins.

Scratch discounts confirmed why no global slot change accompanies #74. Halving
128 to 64 reduced the repeated-reference account to about 452.4 MB but failed
`memory_is_opt_in_and_has_an_exact_portable_edge` (the established portable
overflow edge); the object profile had only 9,960 accounted versus 9,016 heap
bytes. Reducing to 32 failed `representative_shapes_account_for_measured_heap`
with 7,496 accounted versus 9,016 heap bytes. These changes affect every scalar
and container, beyond the Each vector event. Neither candidate satisfies the
requirement to preserve all existing tests and conservative shape coverage.
