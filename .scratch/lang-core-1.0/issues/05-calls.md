# 05 — Calls: functions, actions and built-ins

**What to build:** `@` calls parse and materialize as 1.0 does:
- **`fn` flag:** an `@Name(...)` that isn't a built-in carries `fn: true`. If it isn't in the library's call registry either, it reports `unknown-function` and becomes null, and its statement is kept.
- **Built-ins and steps:** those that aren't lazy carry `mappedProps`, their arguments by param name.
- **Library functions and actions:** their arguments are checked like props (`excess-args`, `missing-required`, `null-required`, `type-mismatch`, defaults). An invalid function call is null, and an invalid action is the empty plan `Action([])`.
- **Query and Mutation arguments:** the calls inside them get their arguments named too.
- **`{steps: [...]}`:** an object literal like this counts as dynamic.

**Blocked by:** 03 — Libraries.

**Status:** done (2026-10-16)

- [x] Every pending case whose difference is in calls conforms, including the fixtures with library functions and actions.
- [x] `scripts/check.sh` passes, with the pending lists shorter.

## Comments

**2026-10-16, from ticket 03:** 1.0's materialize checks the call registry before components. So a name that's both a listed component and a library action (`Badge` in the hand schema `libraries`) materializes as the action even when called without `@`. `hand/102-library-shapes-and-calls` covers it, and it's a candidate quirk to write up.

**2026-10-16, done.** Calls parse and materialize as 1.0's do:
- **The `fn` flag:** `@Name(...)` with a name that isn't a builtin is `Ast::Comp { function: true }`, written `"fn": true` after `args`. A component call excludes those calls (and the entry rule with it).
- **Order:** the call registry comes before components: a function, step or reserved call stays AST for the runtime. An `@` name in neither reports `unknown-function` and is null (`null` in value mode, `{k: "Null"}` in an expression).
- **`mappedProps`:** a builtin that isn't lazy gets its args by param name. A library function or action gets its args checked first (`mapCallArgs`): excess args, missing or null required ones, and each literal arg validated against its param's schema, with defaults filling in and optional invalid ones left out. An invalid function call is `{k: "Null"}`, and an invalid action the empty plan `Action([])`; an action with any error is invalid.
- **Query and Mutation args:** their calls get their args by name, as written (`nameRawCallArgs`), outer calls checked before inner ones.
- **`{steps: [...]}` literals:** count as dynamic.

**The gap:** parse cases pending went from 171 to 113. The statement stage (the `fn` flag in expression trees) has nothing pending now (63 before), and enriched errors went from 129 to 19. None of what's left is about calls:
- 18 one-shot differences are data in component slots (ticket 06), and their enriched errors are the 19 left;
- 95 differ only while streaming, 94 at a half-typed `$name` (ticket 07), and one in a resync sequence, also a component-slot type mismatch.

`hand/102` (the `Badge` case) conforms.

**Added:**
- the `Format` function in the `libraries` schema, and `hand/103-library-function-args`: object, array and boolean args filled from defaults or pruned, a null required arg, a param named like a prototype method, and calls inside a Query's ternary and assignment. Its one-shot result matches; streaming waits on ticket 07's `$name` rule;
- a size test shape: builtin calls nested in a runtime expression, which double with their `mappedProps`.

Coverage is 99.23% of lines, all explained.
