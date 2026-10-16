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

**Status:** done (2026-10-16)

- [x] Every pending merge, serialize, error-hint and prompt case conforms.
- [x] `scripts/check.sh` passes, with the pending lists shorter.

## Comments

**2026-10-16, done.**
- **Converting back to source:** a call in a runtime expression is written with `@` when the library's call registry has it, so the library's functions and actions as well as the built-ins; `Action` still isn't. All 43 pending cases conform.
- **The prompt:**
  - the built-in function and step lines are the reference's generated ones (`@Count(array: any[]) → number`), copied from its `schemaSignature` output;
  - a spec's `functions` are listed after the built-ins, and its `actions` after the built-in steps;
  - `builtinFunctions` lists the built-ins or leaves them out; without them, library functions get a section of their own, and so do library actions when no component takes an action;
  - one step is taught bare and several in `Action([...])`;
  - a restricted slot (`/[:|] @[A-Z]/`, `share?: @CopyToClipboard | @OpenUrl`) counts as taking an action, and adds its own line;
  - the streaming rules number their steps by what's enabled; the built-ins say "on data" without tool calls.
  All 29 pending prompts conform.
- **Error hints:** `severity` was already carried through in ticket 04; nothing was pending.
- **Found in passing:** the prompt read a spec's maps as objects only, where the reference uses `Object.entries`, `values` and `keys`, so components given as an array listed nothing. That gap went back to 0.3.0. The prompt now reads every map that way.

**New prompt specs** (10, with reference goldens): `builtinFunctions` on, off (`0`) and null; library functions alone; library actions alone; restricted slots, and near misses that aren't (`kind?:@Lower`, `at: @lower`, `a |@B`, `user@Example`); and functions, actions and components given as an array or a string.

**The gap:** none. Every pending list is empty: 532 parse cases, 59 prompts, 2,521 merges, 504 trees back to source and 337 enriched error lists conform to 1.0.1. What's left is ticket 09's finish line.
