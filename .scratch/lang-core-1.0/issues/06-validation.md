# 06 — Validation: data in component-only slots

**What to build:** Validation matches 1.0:
- **Data in component-only slots:** a slot whose every option is a `$ref` to a component takes components only. Data there (an object, array, string, number or boolean) is a `type-mismatch`, with 1.0's expected and actual text, and is pruned.
- **No mutation:** 1.0's validation returns cleaned copies instead of changing values in place. The core already copies before changing. Cases written for it confirm the two agree, and the "schema default object shared between parses" divergence is rechecked.

**Blocked by:** 03 — Libraries.

**Status:** done (2026-10-16)

- [x] Every pending case whose difference is in validation conforms.
- [x] The shared-default divergence is either gone from CONFORMANCE.md or still documented with a reason.
- [x] `scripts/check.sh` passes, with the pending lists shorter.

## Comments

**2026-10-16, done.** Validation matches 1.0:
- **Component-only slots (0.3.2):** a slot whose every option of its `$ref`, `anyOf` or `oneOf` is a `$ref` to a component takes components only. Data there is a `type-mismatch`, whose expected text lists the components (`Card | Text`) and whose actual text is `array`, `plain object` or the value's type, and it's pruned. That includes an inherited method read from an object (`function`), as for `isPrototypeOf` in `hand/89`.
- **No mutation:** 1.0 fills a missing required key's default into a copy, and validates the object as it was, so a filled default isn't validated. The core validated it, as 0.3.0 did by filling in place; it doesn't now. Everything else already matched, since the core copies before changing. The divergence "a schema default object, mutated by one parse, shared with later parses" is gone: 1.0 never changes a default. Its row is removed from CONFORMANCE.md.

**The gap:**
- 15 more parse cases conform, and every one-shot result now matches 1.0;
- enriched errors have nothing pending (19 before);
- the 99 parse cases left all first differ at a half-typed `$name` (ticket 07).

**Added:** a `Methods` component in the `libraries` schema with `hand/104-component-only-slots` (an inherited method in a component-only slot and in a union that takes data, a component and a string there), and an API test for the new divergence: a union that isn't an array of objects, where the reference throws (checked).
