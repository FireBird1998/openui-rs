# 03 — Libraries: components, action shapes and the call registry

**What to build:** A component library read from a 1.0 JSON schema means what it means to lang-core 1.0:
- **Components:** when the schema has `properties`, only the names it lists are components, and other `$defs` are data shapes. Without `properties`, every `$def` is a component, as before, except the action shapes.
- **Action shapes:** `ActionExpression`, `OpenUrl`, `ToAssistant` and the library's own actions are never components. Slot schemas that `$ref` them are inlined, so action slots validate as their unions.
- **The call registry:** the built-in functions and steps, plus the library's `functions` and `actions`, each with params compiled like a component's props (order, required, defaults, schema).

**Blocked by:** 02 — Vendor lang-core 1.0.1.

**Status:** ready-for-agent

- [ ] The pending cases whose difference is only which `$defs` are components conform.
- [ ] The library exposes the call registry that tickets 05 and 08 use.
- [ ] `scripts/check.sh` passes, with the pending lists shorter.
