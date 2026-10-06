# Quirks

Behaviour of the reference parser that looks wrong but that the core reproduces anyway, because the contract is to match the reference byte for byte. Fixing a quirk happens upstream first; the core follows when the pinned commit moves.

Each entry has a minimal program, what the reference does, the corpus cases that pin it, and a draft upstream report. Nothing here has been filed yet.

## 1. An apostrophe in unfenced prose swallows the program

- **Found:** 2026-10-06, while porting the preprocessing (ticket 05). Reference commit `bba1d251`.
- **Pinned by:** `hand/78-apostrophe-in-leading-prose`, `hand/79-apostrophe-in-trailing-prose`, and `hand/73-prose-with-apostrophes` (an odd count that happens to survive).

```
Here's the UI:
root = Root([a])
a = Text("x")
```

**Reference behaviour:** `root` is `null`, `statementCount` is 0, and `incomplete` is `true`, so the UI renders nothing. Measured variants:

| Model output | Result |
|---|---|
| `Here is the UI:` then the program | renders |
| `Here's the UI:` then the program (with or without `//` comments) | **nothing**: 0 statements |
| `Here's what I've got:` then the program (two apostrophes) | renders: the two quotes pair up |
| `Here's the UI:` then the program inside a ```` ``` ```` fence | renders: the fence is extracted first |
| the program, then `Here's the UI.` | renders, but `incomplete` stays `true`, so every element is `partial` even after the stream ends |

**Why:** `'` is a string delimiter everywhere: in `autoClose`, in the lexer, and in `stripComments`. Prose isn't removed from unfenced output, so the apostrophe in "Here's" opens a single-quoted string that runs to the end of the text. `autoClose` closes it (so the result is marked incomplete), and the lexer reads the whole program as that one string, leaving no statements. Trailing prose opens the string after the program, which only trips `incomplete`.

**Draft upstream report:**

> **Unfenced model output that starts with an apostrophe ("Here's the UI:") renders nothing.** When a response isn't wrapped in a code fence and the prose before the program has an odd number of apostrophes, `parse()` returns `root: null` with `statementCount: 0`. `autoClose` and the lexer both treat `'` as a string delimiter, so "Here's" opens a single-quoted string that swallows the rest of the program. Prose *after* the program has a milder effect: everything renders but `meta.incomplete` stays `true`, so elements stay `partial` after the stream ends. `stripFences` already ignores apostrophes (see its "does not treat apostrophes as string delimiters" test), but unfenced prose reaches the statement stages untouched. Possible fixes: drop lines that can't start a statement before lexing, or only treat `'` as opening a string where an expression can start. Repro: `parse("Here's the UI:\nroot = Root([a])\na = Text(\"x\")", lib)` → `root: null`.

## 2. Builtins named after `Object.prototype` members count as Query/Mutation calls

- **Found:** 2026-10-06, while porting materialize (ticket 06). Reference commit `bba1d251`.
- **Pinned by:** `hand/47-prototype-named-builtins`.

```
root = Root([@toString(1), @constructor(), Text(@valueOf)])
```

**Reference behaviour:** two `inline-reserved` errors, *"toString() must be declared as a top-level statement, not used inline as a value"* (and the same for `constructor`), and both calls are dropped. Declared as a statement, it's worse: after `a = @toString(1)`, every reference to `a` becomes `{"k":"RuntimeRef","n":"a","refType":"query"}`, but `queryStatements` is empty, so the runtime is asked to run a query that was never declared.

**Why:** `isReservedCall(name)` is `name in RESERVED_CALLS`, and `in` also sees what every object inherits: `toString`, `constructor`, `valueOf`, `hasOwnProperty`, `__proto__` and seven more.

**Draft upstream report:**

> **`isReservedCall` matches `toString`, `constructor` and other `Object.prototype` names.** `isReservedCall` in `packages/lang-core/src/parser/builtins.ts` uses `name in RESERVED_CALLS`, which is true for inherited names. So `@toString(x)` is reported as an inline Query/Mutation, and a statement `a = @toString(x)` turns every reference to `a` into a query `RuntimeRef` with no matching entry in `queryStatements`. Use `Object.hasOwn(RESERVED_CALLS, name)` or a `Set`. Repro: `parse("a = @toString(1)\nroot = Root([Text(a)])", lib)`.

## 3. Prop names that look like numbers take positional args first

- **Found:** 2026-10-06 (ticket 06). This is JavaScript object semantics rather than a bug, but it surprises library authors.
- **Pinned by:** `hand/84-index-like-param-names`, and `hand/44-integer-like-keys` for object literals.

A component's positional order is `Object.keys(properties)`, and JavaScript lists array-index keys (`"0"`, `"10"`) before all others. So with `properties: {"b": ..., "0": ..., "a": ..., "10": ...}`, the order is `0, 10, b, a`, and `Grid("x", "y")` sets `"0"` and `"10"`, not `"b"`. The same reordering applies to `$defs` (the order of "Available components") and to object literals in programs. Worth a note in the docs on defining components, rather than a code change.
