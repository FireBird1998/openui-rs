---
status: accepted
---

# Share matching runtime substitution bodies, preserve their charges

A loop substitutes both positional arguments and mapped props. When those fields
start with the same immutable subtree, retaining two independent transformed
bodies at every nested component doubles live memory at each level. Runtime
component calls consume mapped props, while deferred actions and arbitrary host
scopes can inspect or retain positional syntax.

For a component with one positional argument and one mapped prop, reuse the
transformed argument's backing allocations when the original children have the
same body identity. Both substitution walks still run, in their original order,
and pay their original work and room. Keep the complete positional syntax.
Identity checks inspect a fixed number of header fields; they do not compare
whole trees. Typed ASTs and descriptor data use this same substitution path.

`EvaluationScope::mapped_call_only` opts in to this representation change.
Its default is false. The runtime opts ordinary components in and excludes
host implementations, actions and deferred assignment bodies. This keeps
observable/retained syntax independent of the optimization. Parsing uses the
unchanged policy (no sharing); a zero-sized private policy erases all runtime
policy state from its monomorphization. No runtime concept enters its evaluator.

The allocator regression covers a unique 1,000-byte string leaf, depths 1–8,
one/three rows, typed AST and shared descriptor data. Independently archived
8ffe3b0 results pin output hashes, cut flags and the exact work/room thresholds
for the one-row cases. The one-row additional heap peak is below 128,000 bytes
at every tested level. This is a finite shape bound, not a universal flat peak:
active rewritten templates and traversal metadata still grow with depth.

## Serialized room is not a universal peak-memory bound

ADR 0001 bounds serialized output and work. It does not establish constants in
`peak <= A * result_room + B * supplied_input_bytes + C` for all runtime calls.
In particular, an eager temporary can contain many individually admitted host
results before a later opaque host callback chooses one array index. Within the existing in-memory evaluator, preserving
that uncut output and the callbacks' order/exactly-once behavior requires keeping
all candidates. Their bytes are computed output, not supplied input (ADR 0003).

`temporary_array_semantics` pins this behavior and records allocator peaks.
It deliberately does not require a minimum peak that would reject a future
compact representation. The lower-bound argument concerns arbitrary opaque
payloads, rather than incompressibility of one deterministic fixture.
With n values each about room/4 bytes, live payload is n*room/4 while the program
has O(n log n) bytes. Configurable work permits this family to grow. No constants
independent of the work setting can satisfy the proposed formula. Host allocations
inside callbacks are also outside runtime control. A universal bound requires a
separate temporary-memory allowance that may cut previously uncut programs, or
a broader input/peak contract. Neither is silently introduced here. External spill/paging storage would be a
new I/O, lifetime and failure contract, outside this in-memory runtime decision.

Consequently the repeated-reference `Each(L,i,L)` witness and unread-default
witness remain unresolved. Finite allocator shape bounds in TESTING.md must not
be described as a global live-memory guarantee. Allocator figures measure Rust
heap requests; stacker mappings are accounted separately by ADR 0002.
