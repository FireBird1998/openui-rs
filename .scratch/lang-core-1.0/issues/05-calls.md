# 05 — Calls: functions, actions and built-ins

**What to build:** `@` calls parse and materialize as 1.0 does:
- **`fn` flag:** an `@Name(...)` that isn't a built-in carries `fn: true`. If it isn't in the library's call registry either, it reports `unknown-function` and becomes null, and its statement is kept.
- **Built-ins and steps:** those that aren't lazy carry `mappedProps`, their arguments by param name.
- **Library functions and actions:** their arguments are checked like props (`excess-args`, `missing-required`, `null-required`, `type-mismatch`, defaults). An invalid function call is null, and an invalid action is the empty plan `Action([])`.
- **Query and Mutation arguments:** the calls inside them get their arguments named too.
- **`{steps: [...]}`:** an object literal like this counts as dynamic.

**Blocked by:** 03 — Libraries.

**Status:** ready-for-agent

- [ ] Every pending case whose difference is in calls conforms, including the fixtures with library functions and actions.
- [ ] `scripts/check.sh` passes, with the pending lists shorter.

## Comments

**2026-10-16, from ticket 03:** 1.0's materialize checks the call registry before components. So a name that's both a listed component and a library action (`Badge` in the hand schema `libraries`) materializes as the action even when called without `@`. `hand/102-library-shapes-and-calls` covers it, and it's a candidate quirk to write up.
