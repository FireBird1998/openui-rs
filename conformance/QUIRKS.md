# Quirks

Behaviour of the reference parser that looks wrong but that the core reproduces anyway, because the contract is to match the reference byte for byte. Fixing a quirk happens upstream first; the core follows when the pinned commit moves.

Each entry has a minimal program, what the reference does, the corpus cases that pin it, and a draft upstream report. Nothing here has been filed yet. Every entry was checked again against lang-core 1.0.1 on 2026-10-16.

## 1. An apostrophe in unfenced prose swallows the program

- **Found:** 2026-10-06, while porting the preprocessing (ticket 05). Reference commit `bba1d251`; still so in 1.0.1 (`eb20708`).
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

- **Found:** 2026-10-06, while porting materialize (ticket 06). Reference commit `bba1d251`; still so in 1.0.1 (`eb20708`).
- **Pinned by:** `hand/47-prototype-named-builtins`, `fuzz/3cdbf706`.

```
root = Root([@toString(1), @constructor(), Text(@valueOf)])
```

**Reference behaviour:** two `inline-reserved` errors, *"toString() must be declared as a top-level statement, not used inline as a value"* (and the same for `constructor`), and both calls are dropped. Declared as a statement, it's worse: after `a = @toString(1)`, every reference to `a` becomes `{"k":"RuntimeRef","n":"a","refType":"query"}`, but `queryStatements` is empty, so the runtime is asked to run a query that was never declared.

It's inconsistent, too: choosing the entry statement compares against `"Query"` and `"Mutation"` by name, so `r = @toString(` is still picked as the root statement, and then fails as an inline reserved call (fuzz case `fuzz/3cdbf706`, found in ticket 08).

**Why:** `isReservedCall(name)` is `name in RESERVED_CALLS`, and `in` also sees what every object inherits: `toString`, `constructor`, `valueOf`, `hasOwnProperty`, `__proto__` and seven more.

**Draft upstream report:**

> **`isReservedCall` matches `toString`, `constructor` and other `Object.prototype` names.** `isReservedCall` in `packages/lang-core/src/parser/builtins.ts` uses `name in RESERVED_CALLS`, which is true for inherited names. So `@toString(x)` is reported as an inline Query/Mutation, and a statement `a = @toString(x)` turns every reference to `a` into a query `RuntimeRef` with no matching entry in `queryStatements`. Use `Object.hasOwn(RESERVED_CALLS, name)` or a `Set`. Repro: `parse("a = @toString(1)\nroot = Root([Text(a)])", lib)`.

## 3. Prop names that look like numbers take positional args first

- **Found:** 2026-10-06 (ticket 06). This is JavaScript object semantics rather than a bug, but it surprises library authors. Still so in 1.0.1 (`eb20708`).
- **Pinned by:** `hand/84-index-like-param-names`, and `hand/44-integer-like-keys` for object literals.

A component's positional order is `Object.keys(properties)`, and JavaScript lists array-index keys (`"0"`, `"10"`) before all others. So with `properties: {"b": ..., "0": ..., "a": ..., "10": ...}`, the order is `0, 10, b, a`, and `Grid("x", "y")` sets `"0"` and `"10"`, not `"b"`. The same reordering applies to `$defs` (the order of "Available components") and to object literals in programs. Worth a note in the docs on defining components, rather than a code change.

## 4. Streaming counts redefined statements; one-shot doesn't

- **Found:** 2026-10-06, noted in ticket 01, confirmed in ticket 07. Reference commit `bba1d251`; still so in 1.0.1 (`eb20708`).
- **Pinned by:** `hand/18-redefined-statement` and `hand/85-streaming-redefinition-1127` under every streaming chunking, and 19 more cases the stream-end check lists.

```
root = Text(a)
a = "one"
a = "two"
```

**Reference behaviour:** `parse()` reports `statementCount: 2`. Streaming the same complete text and ending the stream (`getResult(true)`) reports `statementCount: 3`.

**Why:** one-shot uses the number of distinct ids (`stmtMap.size`), while the stream parser increments `completedCount` for every completed statement, redefinitions included. The tree and everything else agree, so this only matters to anything that reads `statementCount`.

**Draft upstream report:**

> **`meta.statementCount` differs between `parse()` and the stream parser when an id is redefined.** For the same final text, `parse()` counts distinct statement ids and `createStreamParser` counts every statement it completed, redefinitions included (`completedCount`), even once the stream has ended (`getResult(true)`). The 1.0 specification says a stream's end must equal a batch parse (language.md, section 4). Either count distinct ids in the stream parser (`completedStmtMap.size`, plus new pending ids), or say in the spec that the streaming count includes redefinitions.

## 5. A mismatched closing bracket leaves the one-shot result "incomplete" forever

- **Found:** 2026-10-06, checking that streaming ends where parsing ends (ticket 09). Reference commit `bba1d251`; still so in 1.0.1 (`eb20708`), with the stream ended by `getResult(true)`.
- **Pinned by:** `hand/71-expression-forms` (via `f = {..., [: 6, 7}`), under every chunking.

```
f = (]
root = Text("x")
```

**Reference behaviour:** `parse()` reports `incomplete: true`, and `root` (and every element) has `partial: true`, even though the text is complete. Streaming the same text ends with `incomplete: false` and `partial: false`.

**Why:** `autoClose` only pops a bracket of the matching type, so `(` closed by `]` stays open and the whole program is treated as cut off mid-stream. The stream parser's statement scanner counts any closing bracket as closing the last one, so it sees the first line as a finished statement. A UI that shows partial elements as loading would show this one loading forever when parsed one-shot.

**Draft upstream report:**

> **One-shot `parse()` marks a complete program `incomplete` (and every element `partial`) after a mismatched closing bracket.** `autoClose` matches bracket types while the stream parser's statement scanner (`scanNewCompleted`) doesn't, so `f = (]\nroot = Text("x")` parses one-shot as incomplete with `partial: true` elements, but streams as complete. Pick one rule for both, likely "any closer closes the innermost opener", which is also what the expression parser effectively does.

**Related:** a stream that ends on a redefinition still being written keeps the earlier definition. That was by design in 0.3; it isn't in the 1.0 specification (quirk 11).

## 6. Merging drops a ternary's continuation lines

- **Found:** 2026-10-06, porting `mergeStatements` (ticket 12). Reference commit `bba1d251`; still so in 1.0.1 (`eb20708`).
- **Pinned by:** merge case `hand/ternary-continuation`.

```
existing:                 patch:
root = Stack([a])         root = Stack([a])
a = $flag
  ? Title("yes")
  : Title("no")
```

**Reference behaviour:** the merged program is `root = Stack([a])` and `a = $flag`. The two continuation lines are gone, so `a` changes meaning from a ternary to the flag itself. The statement wasn't even in the patch.

**Why:** `mergeStatements` cuts the program into statements with its own `splitStatementSource`, which splits at every newline outside brackets. The parser's splitter (and the stream parser's) treats a next line starting with `?` or `:` as a continuation. The two continuation lines become statement texts of their own, have no `name =`, and are dropped.

**Draft upstream report:**

> **`mergeStatements` drops the continuation lines of a multi-line ternary, even in statements the patch doesn't touch.** `splitStatementSource` in `parser/merge.ts` splits at every newline outside brackets, unlike `split` in `statements.ts` and the stream parser's `scanNewCompleted`, which keep a line starting with `?` or `:` with the statement before it. So `a = $flag\n  ? Title("yes")\n  : Title("no")` comes out of any merge as `a = $flag`. The prompt's own syntax rules show ternaries, and models do break long ones across lines. Reuse the parser's continuation rule in `splitStatementSource`. Repro: `mergeStatements('root = Stack([a])\na = $flag\n  ? Title("yes")\n  : Title("no")', 'root = Stack([a])')`.

## 7. An apostrophe in prose before the existing program makes a merge lose it

- **Found:** 2026-10-06 (ticket 12), a sibling of quirk 1. Still so in 1.0.1 (`eb20708`).
- **Pinned by:** merge case `hand/prose-apostrophe-existing`.

```
existing:                 patch:
Here's your UI:           a = Title("B")
root = Stack([a])
a = Title("A")
```

**Reference behaviour:** the merged program is just `a = Title("B")`. `root` and everything else in the existing program are gone.

**Why:** the existing program isn't stripped of fences or prose (only the patch is), and `splitStatementSource` treats `'` as a string delimiter. "Here's" opens a string that runs to the end, so the whole program is one statement text whose first statement can't be parsed. With no existing statements, the merge returns the patch alone.

**Draft upstream report:**

> **`mergeStatements` returns only the patch when the existing program starts with prose containing an apostrophe.** Unlike the patch, `existing` isn't passed through `stripFences`, and `splitStatementSource` treats `'` as opening a string, so "Here's your UI:" swallows the program and no existing statements are found. A host that stores the model's raw first response and merges edits into it loses the whole UI on the first edit. Run `existing` through the same preprocessing as the patch (or as `parse`). Repro: `mergeStatements("Here's your UI:\nroot = Stack([a])\na = Title(\"A\")", 'a = Title("B")')` → `'a = Title("B")'`.

## 8. An id defined twice in the existing program comes out twice

- **Found:** 2026-10-06 (ticket 12). Still so in 1.0.1 (`eb20708`).
- **Pinned by:** merge cases `hand/duplicate-existing`, `hand/duplicate-existing-redefined` and `hand/duplicate-existing-deleted`.

```
existing:                 patch:
root = Stack([a])         b = Title("B")
a = Title("1")
b = Title("2")
a = Title("3")
```

**Reference behaviour:** `root = Stack([a])`, `a = Title("3")`, `a = Title("3")`. The last definition is printed twice, in both places, and the first one's text is lost. A patch `a = null` removes only the first listing; the second disappears only because its text was deleted along with it.

**Why:** the merge keeps a `Map` of id to text and a separate `order` array, and pushes every existing id onto `order`, so a re-defined id is listed twice and both listings look up the latest text.

**Draft upstream report:**

> **`mergeStatements` duplicates a statement that's defined twice in the existing program.** Every existing statement's id is pushed onto `order` (`merge.ts`), so an id defined twice is listed twice and printed twice with its last text. Push an id only the first time it's seen, as the patch loop already does. Repro: `mergeStatements('root = Stack([a])\na = Title("1")\na = Title("3")', 'b = Title("B")')`.

## 9. Serializing drops parentheses around an equal-precedence right operand

- **Found:** 2026-10-06, porting `jsonToOpenUI` (ticket 12). Still so in 1.0.1 (`eb20708`).
- **Pinned by:** serialize case `hand/precedence`.

An expression tree for `a - (b - c)` (a `BinOp` `-` whose right operand is a `BinOp` `-`) serializes as `a - b - c`, which parses back as `(a - b) - c`. The same for `a / (b % c)`, `a % (b * c)`, `a - (b + c)` and `a == (b != c)`.

**Why:** `serializeBinOpChild` adds parentheses only when the child's precedence is *lower* than the parent's. The operators are left-associative, so a right operand of *equal* precedence needs them too. The 1.0 specification says serializers "SHOULD add parentheses where parsing back would change the grouping, as in `a - (b - c)`" (language.md, section 9.2). Quirk 13 is the same gap elsewhere in the serializer.

**Exception to the round-trip invariant** (see TESTING.md): a tree with a `BinOp` whose right operand is a `BinOp` of the same precedence.

**Draft upstream report:**

> **`jsonToOpenUI` changes the meaning of `a - (b - c)`.** `serializeBinOpChild` in `parser/serialize.ts` parenthesizes a child only when `childPrec < parentPrec`, but every binary operator is left-associative, so a right child of equal precedence needs parentheses too: `a - (b - c)` serializes as `a - b - c`, which parses as `(a - b) - c`. The helper already receives `_side`; parenthesize when `side === "right" && childPrec === parentPrec`. Repro: serialize an element whose prop is `{k: "BinOp", op: "-", left: Ref a, right: {k: "BinOp", op: "-", left: Ref b, right: Ref c}}`.

## 10. Serialized object keys aren't quoted

- **Found:** 2026-10-06 (ticket 12). Still so in 1.0.1 (`eb20708`), where it's by design.
- **Pinned by:** serialize case `hand/values`.

A plain object prop `{"a b": 1, "": 2}` serializes as `{a b: 1, : 2}`, which doesn't parse back. (A key with quotes in it, `"q"`, happens to come out as valid syntax.)

**Why:** `serializeValue` writes `${k}: ...` for object entries, and the `Obj` AST branch does the same, with no quoting. Object literals in OpenUI Lang accept quoted keys, so `JSON.stringify(k)` would always work.

**By design since 1.0:** the specification says "Object keys are written unquoted, so they must be valid names to round-trip" (language.md, section 9.2). So the draft report below is withdrawn rather than waiting to be filed; at most, upstream might mention it in the serializer's docs. The entry stays because the core reproduces the behaviour.

**Exception to the round-trip invariant:** a tree with an object key that isn't a valid bare name (an identifier, a `$name`, or digits without a leading zero).

**Draft upstream report (withdrawn):**

> **`jsonToOpenUI` writes object keys without quotes, so keys that aren't identifiers produce text that doesn't parse.** Both `serializeValue` (plain objects) and the `Obj` branch of `serializeASTNode` emit `${k}: ...`. A prop like `{"Content-Type": "json"}` or `{"first name": "x"}` round-trips to invalid OpenUI Lang. Quote keys that aren't plain identifiers (or always: the parser accepts `{"a": 1}`). Repro: serialize `Slot({"a b": 1})`.

## 11. At a stream's end, a final redefinition that needed closing doesn't win

- **Found:** 2026-10-16, reviewing stream ends for lang-core 1.0 (1.0 ticket 07). Reference commit `eb20708` (1.0.1).
- **Pinned by:** `hand/85-streaming-redefinition-1127` under every chunking, and the stream-end check.

```
root = Text(a)
a = "one"
a = "two
```

**Reference behaviour:** `parse()` gives `a` the value `"two"`. Streaming the same text and ending the stream with `getResult(true)` keeps `"one"`.

**Why:** `currentResult` skips a pending statement whose id is already completed when the pending text needed closing (`if (completedStmtMap.has(s.id) && wasIncomplete) continue;`), and `complete` changes nothing there: it only decides how a trailing `$name` is read. In 0.3 this was the upstream #1127 fix working as intended ("existing IDs are replaced only when the pending expression no longer needs automatic closing"). The 1.0 specification now says otherwise: "At stream end a final redefinition wins" (language.md, section 2.4), and the end of the stream "finalizes the pending tail: implicit closing applies, and the resulting statements become completed, including a redefinition of an earlier name" (section 4, rule 7).

**Draft upstream report:**

> **Ending a stream keeps an earlier definition instead of a final redefinition that needed closing, unlike `parse()` and the 1.0 spec.** With `complete`, `createStreamParser` still skips a pending statement whose id is already completed if the pending text needed implicit closing, so the stream's end keeps the earlier definition while `parse()` of the same text uses the last one. language.md, section 2.4 ("At stream end a final redefinition wins") and section 4, rule 7 say the end of the stream finalizes the pending tail. Apply the `wasIncomplete` guard only while `complete` isn't set. Repro: push `root = Text(a)\na = "one"\na = "two` and call `getResult(true)`: `a` is `"one"`, while `parse()` gives `"two"`.

## 12. A library function or action called without `@` is still called, even over a component of that name

- **Found:** 2026-10-16, porting 1.0's call registry (1.0 ticket 03), and confirmed reviewing quirks for 1.0 (1.0 ticket 09). Reference commit `eb20708` (1.0.1).
- **Pinned by:** `hand/106-bare-library-calls`, and `hand/102-library-shapes-and-calls` (`b = Badge("x")`).

```
root = Page([w, b])
w = Widget(Twice(2))
b = Badge("x")
```

With `Twice` a library function, `Badge` both a component and a library action.

**Reference behaviour:** `Twice(2)` is the function call, as `@Twice(2)` is, only without the `fn` flag. `Badge("x")` is the action, not a `Badge` element. A bare built-in is different: `Count(2)` reads as the reference `Count`, and its argument is dropped with an `excess-args` error.

**Why:** materializing a call checks `isRuntimeCall(name)`, which looks the name up in the whole call registry (built-ins, the library's functions and actions) whether or not the call was written with `@`, and before it looks for a component. The expression parser already turns bare built-in names into references, so only library calls get through. The 1.0 specification says otherwise: "Built-ins and custom functions need the `@`. [...] Only `Action([...])` works with or without `@`" (language.md, section 1.5), and an action is "a `@` call" (section 6.3). It forbids a library function named like a component (section 3.6), but not an action, so the `Badge` case is a valid library whose component can't be used.

**Draft upstream report:**

> **A call without `@` to a library function or action is treated as that call, even when a component has the same name.** In `materialize.ts`, the `Comp` case checks `isRuntimeCall(name, ctx)` (the call registry, built-ins plus the library's `functions` and `actions`) for every call, not only for `@` calls (`node.fn`), and before the component lookup. So `Twice(2)` runs the library function, and with a component and an action both named `Badge`, `Badge("x")` gives the action instead of the component. language.md, section 1.5 says custom functions need the `@` and only `Action([...])` works without it, and section 6.3 defines an action as a `@` call. Treat a call without `@` as a component call unless it's `Action`, Query or Mutation, and report an `unknown-component` when there's no component of that name. Repro: with `functions: {Twice: ...}` and `actions: {Badge: ...}` beside a component `Badge`, `parse("root = Page([Widget(Twice(2)), Badge(\"x\")])", lib)` gives a `Twice` call and a `Badge` action.

## 13. Serializing drops the parentheses an operand needs

- **Found:** 2026-10-16, by the round-trip invariant (testing ticket 08). Reference commit `eb20708` (1.0.1).
- **Pinned by:** `hand/107-operands-that-need-parentheses`, through the corpus's serialization of every parse case's tree.

```
root = Root([a, b, c, d, e])
a = Text(!($on && $off))
b = Text(-($x + 1))
c = Text(-(3))
d = Text(($on ? "a" : "b").length)
e = Text($on || ($off ? "a" : "b"))
```

**Reference behaviour:** converting the tree back to source gives `!$on && $off`, `-$x + 1`, `-3`, `$on ? "a" : "b".length` and `$on || $off ? "a" : "b"`. Each parses back as a different expression: `(!$on) && $off`, `(-$x) + 1`, the number literal `-3` (which, unlike the expression, is type-checked, so a `Text` that needs a string is dropped), `$on ? "a" : ("b".length)` and `($on || $off) ? "a" : "b"`. A member access on a number has the same problem: `(5).x` comes back as `5.x`, and a field that's digits, `5. 1`, as the decimal `5.1`.

**Why:** in `serializeASTNode`, `UnaryOp` writes `${op}${operand}`, `Member` and `Index` write the object as it is, and `serializeBinOpChild` parenthesizes only a child that's a lower-precedence `BinOp`, never a `Ternary`. The 1.0 specification asks for these parentheses: "Serializers SHOULD add parentheses where parsing back would change the grouping, as in `a - (b - c)` and `-(a + b)`" (language.md, section 9.2). Quirk 9 is the binary-operator case of the same gap.

**Exception to the round-trip invariant:** a tree with a `UnaryOp` whose operand is a `BinOp`, `Ternary` or `Assign` (or, for `-`, starts with a number); a `BinOp` with a `Ternary` or `Assign` operand; or a `Member` or `Index` whose object is one of those, a `UnaryOp`, or starts with a number, or whose field isn't a name.

**Draft upstream report:**

> **`jsonToOpenUI` drops parentheses that operands need, so `!(a && b)` comes back as `!a && b`.** In `parser/serialize.ts`, `serializeASTNode` writes a `UnaryOp` as `${op}${operand}` and a `Member` or `Index` object as it is, and `serializeBinOpChild` parenthesizes only a lower-precedence `BinOp` child. So `!($on && $off)` serializes as `!$on && $off`, `-($x + 1)` as `-$x + 1`, `($on ? "a" : "b").length` as `$on ? "a" : "b".length`, `$on || ($off ? "a" : "b")` as `$on || $off ? "a" : "b"`, and `-(3)` as `-3`, which parses as a number literal and is then type-checked. Each parses back as a different expression, and language.md, section 9.2 says serializers should add parentheses where parsing back would change the grouping. Wrap a unary operand, a member access's or index's object, and a binary operator's operand in parentheses when they aren't primary expressions (and when a number would absorb the `-` or the `.`). Repro: `jsonToOpenUI(parse('root = Text(!($on && $off))', lib).root, library)` gives `root = Text(!$on && $off)`.

## 14. A call made null when parsing doesn't count as a null prop (candidate)

- **Found:** 2026-10-16, by the round-trip invariant (testing ticket 08). Reference commit `eb20708` (1.0.1).
- **Pinned by:** `hand/108-call-made-null`, through the corpus's serialization of every parse case's tree.
- **Status:** a candidate for the maintainer to judge. The specification's round trip is "up to the recovery rules of section 8", which may cover it.

```
root = Page(@Format(5))
```

With `Format(value: string, ...)` a library function whose `value` is required, and `Page`'s `children` required.

**Reference behaviour:** the literal argument is checked when parsing (`type-mismatch`), and the call becomes null, as 1.0 says it should. But it becomes the expression `{k: "Null"}`, not the value `null`, so `children` holds an expression and `Page` is kept, with no `null-required`. Converting the tree back to source writes that expression as `null`, and `root = Page(null)` drops `Page` with `null-required`. The same null drops the component when it's written as a literal and keeps it when a call evaluates to it.

**Why:** `mapCallArgs` failing gives `{k: "Null"}` (`materialize.ts`), and the required-prop check looks for `null` values, not for null expressions. language.md, section 3.6 says an invalid required argument "makes the call evaluate to null", and section 8.2 that a required prop that is null drops the component (`null-required`).

**Exception to the round-trip invariant:** a tree that holds a `{k: "Null"}` node.

**Draft upstream report:**

> **A required prop filled by a call that parsing already made null isn't reported, while a literal `null` there is.** When a library call's literal arguments fail their check, the call becomes `{k: "Null"}` in the tree. A component whose required prop is that expression is kept, with no `null-required`, so `parse('root = Page(@Format(5))', lib)` keeps `Page` (with `type-mismatch` for the argument), while `parse('root = Page(null)', lib)` drops it. Converting the first tree back to source gives the second program. Either treat a `{k: "Null"}` prop as null in the required-prop check, or keep it as an expression on purpose and say so in language.md, section 8.2.

