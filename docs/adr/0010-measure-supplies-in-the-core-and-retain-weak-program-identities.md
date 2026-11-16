---
status: accepted
---

# Measure supplies in the core and retain weak program identities

The core owns supplied-input measurement. `ExpressionEvaluator::supply_input`
accepts one current program, one starting action/message/form contribution and
one reference-preparation receipt. Each category appears once. Program and
starting receipts carry their measuring limits; using one under different
limits is a typed refusal before accounting changes. No API accepts a reference
byte claim or adds arbitrary receipts together. Receipts retain only scalar
measurements, never supplied values or identities.

The floor remains the largest combined supply seen by the run. Measurement is
caller setup, not evaluation work. Raising the floor earns only its increase;
spent work, fixed limits and hard ceilings remain unchanged (ADR 0003).
References use drawing's exact preparation walk and shared identity set. Refused
reference entries earn nothing and raise their cut only when read. Single
reference tokens measure their own full JSON bytes and logical read height;
preparation does not impose expression depth on an unvisited subtree. Whole-value
consumption still checks the evaluation depth at the actual use.

Program credit has one canonical core measure shared by drawing and actions.
Drawing's historical setup-refusal classification is preserved with a second
bounded core check, which supplies no credit. Fusing those walks is a future
optimization requiring identical flags and caller costs; reusing the old
runtime's independent byte count is not permitted. Partial measurement keeps
independent fitting input and bounded structural prefixes when a deep branch
cannot be visited. Saturation remains bounded. A large branch before or after
a refused one cannot erase the other branch's credit.

A shared-program cache retains a Weak identity, its measurement and statement
indexes. While an input borrows an Arc it cannot mutate that allocation.
Between calls, Arc::make_mut on a uniquely held program moves the value and
invalidates its weak identities without cloning the program. A stale or foreign
allocation cannot reuse the cache. This preserves immutable-cache safety while
removing the suspended run's strong program owner. Only the current indexes and
receipt remain; supplied references and previous programs are not accumulated.

Query defaults stay lazy and frozen for the action run, including after program
replacement. This is the reference harness's per-run context, not React effect
scheduling. Oversized messages unused by a step-array plan are measured but not
copied or retained; default/legacy events still require a bounded message.
Scalar payload inputs remember the latest value per name and earn credit when
that value changes, including changing back. Consecutive equal reads earn no
new credit. Comparison is exact, including string contents and number bits;
there is no digest collision or accumulated value history. Keys earn credit
once per name per run. Every output copy continues to spend work and room.

`supplied_input_accounting`, `action_supply_allowance`, `action_resume_inputs`,
`accounting_followups`, `prepared_reference_compatibility` and `thread_safety`
pin these decisions. General temporary-memory bounds remain deferred under
ADR 0008.

Drawing retains its historical setup saturation guard separately from input
credit. `drawing_keeps_the_existing_saturated_setup_bound_before_unread_deep_declarations`
pins a caller-built program with a 44 MB U+100000 string and an unread
2,000-level array. In main's visitation order the six-unit high-codepoint
estimate saturates the guard before the deep array, so the program completes;
reversing declarations reaches the depth cut. Using actual JSON string bytes
for this guard would cut the formerly completed ordering. That estimate remains
only in `program_setup`'s refusal check; it never grants input credit. Program
credit itself uses one core measure shared by drawing and action runs. Removing
or modernizing the historical guard is deferred because it conflicts with
main's completed results.

The binding-owned `PreparedReferenceValues::owned_with_limits` constructor keeps
its public signature and owns the same core-issued receipt as borrowed
preparation. Moving the source into a Cow does not repeat measurement or accept
a caller size. Session/FFI/wasm call sites remain unchanged. Host callback JSON
returns follow main's computed-result path through core body measurement,
validation and projection; they earn no fresh input allowance. Query/tool data
supplied as references earns normal verified input credit. ADR 0009 separately
owns the host callback JSON and reentry contract.
