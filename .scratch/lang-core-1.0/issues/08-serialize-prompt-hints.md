# 08 — Converting back to source, the prompt and error hints

**What to build:** The rest of the language layer matches 1.0:
- **Converting back to source:** every call in the library's call registry is written with `@`, except `Action`.
- **The prompt:**
  - built-in function and step lines are 1.0's generated ones;
  - a spec's `functions` and `actions` are listed, and there's a `builtinFunctions` option;
  - one step is taught bare and several in `Action([...])`, and restricted action slots are recognized;
  - the streaming rules number their steps by what's enabled.
- **Error hints:** carry `severity` through.

**Blocked by:** 03 — Libraries.

**Status:** ready-for-agent

- [ ] Every pending merge, serialize, error-hint and prompt case conforms.
- [ ] `scripts/check.sh` passes, with the pending lists shorter.
