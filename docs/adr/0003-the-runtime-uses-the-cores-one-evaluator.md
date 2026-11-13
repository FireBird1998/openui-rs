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
data, such as query results, must be included in the caller's initial input
size. This keeps ADR 0001's amplification bound relative to input, rather than
to intermediate results.

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
