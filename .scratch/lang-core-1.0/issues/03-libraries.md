# 03 — Libraries: components, action shapes and the call registry

**What to build:** A component library read from a 1.0 JSON schema means what it means to lang-core 1.0:
- **Components:** when the schema has `properties`, only the names it lists are components, and other `$defs` are data shapes. Without `properties`, every `$def` is a component, as before, except the action shapes.
- **Action shapes:** `ActionExpression`, `OpenUrl`, `ToAssistant` and the library's own actions are never components. Slot schemas that `$ref` them are inlined, so action slots validate as their unions.
- **The call registry:** the built-in functions and steps, plus the library's `functions` and `actions`, each with params compiled like a component's props (order, required, defaults, schema).

**Blocked by:** 02 — Vendor lang-core 1.0.1.

**Status:** done (2026-10-16)

- [x] The pending cases whose difference is only which `$defs` are components conform.
- [x] The library exposes the call registry that tickets 05 and 08 use.
- [x] `scripts/check.sh` passes, with the pending lists shorter.

## Comments

**2026-10-16, done.** `Library::from_json_schema` compiles a schema as 1.0's `compileSchema` does:
- **Components:** the `$defs` that a truthy `properties` names, or every `$defs` entry but the action shapes.
- **Action shapes:** `ActionExpression`, `OpenUrl`, `ToAssistant` and the schema's own actions, when `$defs` has them and `properties` doesn't name them as components. A param schema's `$ref` to one is replaced by the shape, itself inlined, so action slots validate as their unions and their signatures print no type, as 1.0's do.
- **The call registry:** `Library::call` and `Library::calls`. The 20 built-ins come first, in the reference's order, with param names, a kind (function or action) and whether they're lazy; then `functions`, then `actions`, with params compiled like props. Tickets 05 and 08 use it.

**The gap:** 10 more parse cases and 3 error hints conform. Every remaining difference these covered is gone: the components list in `unknown-component` messages, and action-slot signatures. The enriched-errors test now passes the runner's own component names, `Object.keys($defs)`, which include action shapes.

**Added:**
- the hand schema `libraries` with `hand/102-library-shapes-and-calls`: a `properties` that leaves a `$def` out, a custom action's shape inlined into a slot, and a name that's both a listed component and an action;
- three API tests: the registry and inlining, the schemas the reference throws on, and an array `$defs` read by index (checked against the reference).

**Divergences documented:** a call without `params` (the reference throws a `TypeError`), and an action shape that refers to itself (the reference overflows its stack).

**Found, for ticket 05:** a name that's both a component (listed in `properties`) and a library action is materialized as the action when called bare: `Badge("x")` gives a call with `mappedProps`, not an element. 1.0's materialize checks the call registry before components. `hand/102` stays pending until then. It may be a quirk worth writing up.
