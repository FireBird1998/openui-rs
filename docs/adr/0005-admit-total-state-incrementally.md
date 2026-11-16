---
status: accepted
---

# Count total state at each retained name

The runtime Store uses its retained core `Limits` for admission on every write
route. The value and total-state cap are `Limits::result_size()`, the configured
ceiling (8 MiB by default), independent of a drawing program's scaled room.
Using a fixed Store cap avoids silently making previously admitted state invalid
when a shorter program is drawn or an action resumes with new program input.

Total state is the exact JSON object size of the union of snapshot names and
pristine-only names, including escaped keys, colons, commas and braces. Undefined
and pristine-only values count as null. This is the admission representation;
exported snapshot JSON omits undefined entries like JSON.stringify and the
bindings, while the raw snapshot retains those names. Empty state costs zero, allowing empty
stores even with a zero-byte cap. Aliases count at each occurrence, including
aliases within a value, as serialization does. Metadata retained solely for
undefined form wrappers preserves its existing identity semantics on disposal;
it is not another state name.

Keep one measured byte count per name and a running sum. A write measures only
its candidate name and value through the core's bounded measurement, then
subtracts the retained old count and adds the new one using saturating arithmetic.
An overflowing candidate or object-punctuation sum is refused, including when
the configured ceiling itself is usize::MAX. Reset preflight removes all old
counts before summing candidates and restores its exact final sum after applying
the writes, even if an intermediate replacement order saturated the counter.
There is no whole-state serialization on admission. Scalar JSON lengths use the
core helpers directly; already admitted no-ops reuse their retained count.
Snapshot copy-on-write still copies keys when an embedder holds an old snapshot;
that existing cost is separate from admission accounting.

Refusals are typed Store errors and leave the name's value, pristine status,
snapshot identity and version unchanged. Binding/form/action routes use the same
Store path in addition to their evaluation allowance. Reset preflight checks the
combined final state and version before any target changes; a refused reset has
no effects. Reset application uses the core-measured preflight receipts and does
not measure again after mutation starts: changed ownership must not create a
second measurement refusal after a reset prefix has been applied. A smaller
replacement passes total-state admission, but still must
pass the value's own depth/work/size admission.

Initialization loads what fits in JavaScript input order and reports every
refused entry, preserving the existing per-name InitializationResult contract.
This supports streaming defaults without discarding independent accepted values.
Persisted duplicate names are normalized before any pristine changes, so a refused
last occurrence cannot clear a flag through an earlier duplicate. Refusing the
whole persisted supply would require a different atomic batch API and would
break this existing contract.

Core measurement, not a caller-provided size, determines a value's serialized
bytes. The raw UTF-8 name bytes and core value measurement share one admission
work budget; the name scan is deducted before measuring the value. Its work budget uses the Store's fixed-work value or the configured scaled
work ceiling, and its nesting/stack policy follows the public expression
interface. Action runs still have the one run-wide evaluation allowance of ADR
0003; Store admission does not replace or refresh it.

The regression tests are `state_admission.rs` and `public_call_sequences.rs`.
This decision establishes a state cap. It does not establish a general bound on
the peak heap used by intermediate evaluation structures.
