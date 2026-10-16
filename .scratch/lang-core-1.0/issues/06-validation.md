# 06 — Validation: data in component-only slots

**What to build:** Validation matches 1.0:
- **Data in component-only slots:** a slot whose every option is a `$ref` to a component takes components only. Data there (an object, array, string, number or boolean) is a `type-mismatch`, with 1.0's expected and actual text, and is pruned.
- **No mutation:** 1.0's validation returns cleaned copies instead of changing values in place. The core already copies before changing. Cases written for it confirm the two agree, and the "schema default object shared between parses" divergence is rechecked.

**Blocked by:** 03 — Libraries.

**Status:** ready-for-agent

- [ ] Every pending case whose difference is in validation conforms.
- [ ] The shared-default divergence is either gone from CONFORMANCE.md or still documented with a reason.
- [ ] `scripts/check.sh` passes, with the pending lists shorter.
