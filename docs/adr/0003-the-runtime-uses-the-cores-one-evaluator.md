---
status: accepted
---

# The runtime is its own crate over the core's one evaluator

Live state, evaluated trees, bindings and action plans belong in a second
crate, `openui-runtime`, which depends on `openui-lang`. The core stays a
parser that builds and tests on its own. An embedder that only parses needs
no runtime machinery.

There is one evaluator, in the core. Parsing already needs its JavaScript
operators, coercions, built-in functions and `@Each` substitution for state
defaults. The runtime uses it through a public interface over a scope the
caller supplies, with `Limits` and a default. The scope resolves names and
handles calls whose arguments must sometimes remain unevaluated: a set step
keeps its expression until click time, and a binding needs its target.

One allowance can be shared across a tree. Resolving statements and evaluating
many props must not give each one fresh work and result room. Re-entering that
allowance shares its limits; nested evaluations also respect the enclosing
depth restriction. Cuts return flags, and limits count steps and bytes, never
time, under ADRs 0001 and 0002. Parsing keeps its own behaviour and cost.

Supplied state and extra scope count as input when read. Reference and call
results do not add input credit: wrapping computed output cannot increase the
allowance. Statements are paid for by the program's size. External reference
data, such as query results, must be admitted as supplied input before use.
New program/reference input on resume earns credit before it is read; spent
work is never refreshed. This keeps ADR 0001's amplification bound relative to
input, rather than to intermediate results.

An action run has one evaluation-work allowance across steps and mutation
continuations. Input measurement is the caller's setup cost, never evaluation
work: neither a fresh run nor a resume debits it. Runs, resumes and drawing
prepare reference supplies through `PreparedReferenceValues::new`/`with_limits`:
one walk, one shared identity set, one work budget, identical ceilings and
per-entry results. Successful entries earn exactly drawing's total input credit;
refused entries earn none and raise their recorded cut only if read. Unread
large/deep entries cannot refuse an unrelated action. No entry gets a separate
preparation budget or a lazy re-measurement. Prepared receipts retain only measurements; the reference receipt is produced
by that same preparation walk, without accepting a size claim or re-walking data.
Computed values from scope callbacks must never be admitted as supplied input.

For each supply, combine the current program and reference measurements with
the retained starting action/message/form-name measurement. The supplied-input
floor is the maximum of those combined sizes, not independent maxima for the
parts and not a sum over successive supplies. The core raises work and result
allowances only by the difference earned by that floor. Already-spent evaluation
work stays spent. State reads retain their normal credit. With that state
credit, remaining work is `work_for(current floor + state credit) - spent work`.
Per-write/event output room is released between operations; hard ceilings remain.
This replaces the historical spec's steps-times-per-evaluation rule.

A supplied action is bounded evaluated-tree output retained for a later run.
Its input measurement earns credit once as part of the run's starting input;
wrapping or re-supplying it cannot accumulate credit. References are measured as
one current supply, with shared bodies counted once within that measurement.
No identity memo, reference supply, or strong owner for supplied references is
retained by the suspended run. Consequently equal, alternating and growing
supplies have the same limits as a fresh run with the largest combined supply,
short of evaluation work already spent. Borrowed/raw and shared/prepared forms
have identical effects and cut points. Caching changes only caller setup cost.

ADR 0010 consolidates the program byte measure in the core for drawing and
runs, while retaining drawing's existing setup-refusal classification through
a bounded check. Partial program/action measurements retain independently
measured input rather than erasing an entire supplied item. An over-deep kept
value still cuts when evaluated, preserving earlier effects. References reuse
drawing's preparation, including cached read representations. Prepared types
and Store remain Send + Sync. Parser AST measurement memoizes shared
allocations rather than every borrowed node. Program measurement remains
independent of reference preparation.

## Considered options

- **Put the runtime in the core.** Parse-only embedders would carry live-state
  and action machinery, obscuring the core's boundary.
- **Move the evaluator into the runtime.** State defaults would make the core
  depend on the runtime, reversing the dependency.
- **Copy the evaluator.** Semantics and limit fixes would have to be applied
  twice, and the copies would drift.
- **Only accept maps of values.** Resolving statements and calling library
  functions need caller code. A scope supports both without choosing a store.
- **Give each prop a fresh allowance.** It would bound one expression while
  leaving the whole tree unbounded.
