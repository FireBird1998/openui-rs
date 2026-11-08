# Quirks

Behaviour of the reference parser that looks wrong but that the core reproduces anyway, because the contract is to match the reference byte for byte. Fixing a quirk happens upstream first; the core follows when the pinned commit moves.

Each entry has a minimal program, what the reference does, the corpus cases that pin it, and a draft upstream report (21's is drafted as an issue of its own, and 22 has none). Nothing here has been filed yet. Every entry was checked again against lang-core 1.1.0 on 2026-11-06. The cases that pin each one match 1.1's results, so each still holds, except 6, 8 and 20, which 1.1 fixed. Quirks that upstream fixed are kept, under their numbers, in [Fixed upstream](#fixed-upstream).

## 1. An apostrophe in unfenced prose swallows the program

- **Found:** 2026-10-06, while porting the preprocessing (ticket 05). Reference commit `bba1d251`; still so in 1.0.1 (`eb20708`) and in 1.1.0 (`47b9cae`), whose new fence rules don't touch unfenced prose.
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

**Exception to the round-trip invariant** (and merging a program's own source): a tree with a call named after an `Object.prototype` member.

**Draft upstream report:**

> **`isReservedCall` matches `toString`, `constructor` and other `Object.prototype` names.** `isReservedCall` in `packages/lang-core/src/parser/builtins.ts` uses `name in RESERVED_CALLS`, which is true for inherited names. So `@toString(x)` is reported as an inline Query/Mutation, and a statement `a = @toString(x)` turns every reference to `a` into a query `RuntimeRef` with no matching entry in `queryStatements`. Use `Object.hasOwn(RESERVED_CALLS, name)` or a `Set`. Repro: `parse("a = @toString(1)\nroot = Root([Text(a)])", lib)`.

## 3. Prop names that look like numbers take positional args first

- **Found:** 2026-10-06 (ticket 06). This is JavaScript object semantics rather than a bug, but it surprises library authors. Still so in 1.0.1 (`eb20708`).
- **Pinned by:** `hand/84-index-like-param-names`, and `hand/44-integer-like-keys` for object literals.

A component's positional order is `Object.keys(properties)`, and JavaScript lists array-index keys (`"0"`, `"10"`) before all others. So with `properties: {"b": ..., "0": ..., "a": ..., "10": ...}`, the order is `0, 10, b, a`, and `Grid("x", "y")` sets `"0"` and `"10"`, not `"b"`. The same reordering applies to `$defs` (the order of "Available components") and to object literals in programs. Worth a note in the docs on defining components, rather than a code change.

## 4. Streaming counts redefined statements; one-shot doesn't

- **Found:** 2026-10-06, noted in ticket 01, confirmed in ticket 07. Reference commit `bba1d251`; still so in 1.0.1 (`eb20708`).
- **Pinned by:** `hand/18-redefined-statement`, `hand/85-streaming-redefinition-1127` and `hand/145-an-empty-redefinition-after-a-stray-bracket` under every streaming chunking, and 19 more cases the stream-end check lists.

```
root = Text(a)
a = "one"
a = "two"
```

**Reference behaviour:** `parse()` reports `statementCount: 2`. Streaming the same complete text and ending the stream (`getResult(true)`) reports `statementCount: 3`.

**Why:** one-shot uses the number of distinct ids (`stmtMap.size`), while the stream parser increments `completedCount` for every completed statement, redefinitions included. The tree and everything else agree, so this only matters to anything that reads `statementCount`.

**Exception to the chunking and stream-end invariants:** a program that defines a name twice, when the results differ in `statementCount`. That includes a line defining a name again with nothing after its `=`, behind a stray opening bracket: `[`, `a=("")`, `a=` streams to a count of 2 and parses to 1. The invariants found that shape twice in a million programs against 1.1 (lang-core 1.1 ticket 08, candidate 3), and 1.0.1's run had it too; the maintainer decided on 2026-11-07 that it's this quirk, not a new one.

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

**Exception to the chunking and stream-end invariants:** a program with a mismatched closing bracket.

**Draft upstream report:**

> **One-shot `parse()` marks a complete program `incomplete` (and every element `partial`) after a mismatched closing bracket.** `autoClose` matches bracket types while the stream parser's statement scanner (`scanNewCompleted`) doesn't, so `f = (]\nroot = Text("x")` parses one-shot as incomplete with `partial: true` elements, but streams as complete. Pick one rule for both, likely "any closer closes the innermost opener", which is also what the expression parser effectively does.

**Related:** a stream that ends on a redefinition still being written keeps the earlier definition. That was by design in 0.3; it isn't in the 1.0 specification (quirk 11).

## 7. An apostrophe in prose before the existing program makes a merge lose it

- **Found:** 2026-10-06 (ticket 12), a sibling of quirk 1. Still so in 1.0.1 (`eb20708`) and 1.1.0 (`47b9cae`): the merge now splits with the parser's lexer, which reads the apostrophe as opening a string too.
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

**Exception to the chunking and stream-end invariants:** a program that defines a name twice, when the results differ otherwise.

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

**Exception to the round-trip invariant** (and merging a program's own source): a bare library call, which comes back with `@` and the `fn` flag.

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

## 15. Merging puts what it appends after an unfinished statement, which takes it in

- **Found:** 2026-10-16, by the merge invariants (testing ticket 09). Reference commit `eb20708` (1.0.1); still so in 1.1.0 (`47b9cae`).
- **Pinned by:** merge case `hand/into-an-unfinished-statement`.
- **Status:** a candidate for the maintainer to judge.

```
existing:  root = Root([a, b])
           a = Text("one")
           b = Text("cut off
patch:     c = Text("new")
           root = Root([a, b, c])
```

**Reference behaviour:** the merge gives `root = Root([a, b, c])\na = Text("one")\nb = Text("cut off\nc = Text("new")`. Parsed, `b`'s string runs to the end and takes `c` in: `b` is `"cut off\nc = Text("`, and there's no `c`. A reply cut off mid-string (a token limit, a dropped connection) followed by an edit loses the patch's new statements. Merging the same patch again changes the program again, so merging isn't idempotent either.

**Why:** `mergeStatements` copies each statement's text as it is and joins them with newlines. A statement that ends inside a string or a bracket, in the existing program or in the patch, reads on into whatever comes after it. language.md, section 7 says the merge reads both texts with sections 1.2 and 1.4, which is how it finds the statements, but not what to write for one that isn't finished.

**Exception to the merge invariants:** a program or patch with an unclosed string or bracket, or a statement line whose quotes or brackets don't balance.

**Draft upstream report:**

> **Merging a patch into a program whose last statement is unfinished loses the patch's new statements.** `mergeStatements` writes each statement's text as it was and appends new ones after it, so after `b = Text("cut off`, an appended `c = Text("new")` sits inside `b`'s unclosed string: `mergeStatements('root = Root([a, b])\na = Text("one")\nb = Text("cut off', 'c = Text("new")\nroot = Root([a, b, c])')` parses with no `c`. Close an unfinished statement as parsing would (`autoClose`) before writing it, or drop it, so what follows stays separate.

## 16. Merging into an empty program returns the patch as it is

- **Found:** 2026-10-16, by the merge invariants (testing ticket 09). Reference commit `eb20708` (1.0.1); still so in 1.1.0 (`47b9cae`), which returns the patch's statements joined, as before.
- **Pinned by:** merge case `hand/into-an-empty-program`.
- **Status:** a candidate for the maintainer to judge.

```
existing:  (empty)
patch:     root = Root([a])
           a = Text("A")
           b = Text("B")
           b = null
           a = Text("A2")
```

**Reference behaviour:** the merge is the patch, line for line: `b = null` stays a statement instead of deleting `b`, `a` is defined twice, and `b`, which `root` can't reach, isn't removed. Merging the same patch into a program that has statements gives `root = Root([a])\na = Text("A2")`. So merging isn't idempotent: the second merge cleans up what the first left. An existing program the merge finds no statements in (prose, a comment, or quirk 7's apostrophe) takes the same path.

**Why:** `mergeStatements` returns early, `if (!existingStmts.length) return patchStmts.map((stmt) => stmt.raw).join("\n")`, before the deletions, the duplicates and the reachability pass. language.md, section 7 asks for all three: "`name = null` in a patch deletes the statement", "The merged program has each name once", and statements `root` can't reach are removed.

**Exception to the merge invariants:** an existing program, or a first merge's result, with no statements.

**Draft upstream report:**

> **`mergeStatements` into an empty program skips deletions, duplicates and the reachability pass.** With no existing statements it returns the patch's statements as they are, so `mergeStatements("", 'root = Root([a])\na = Text("A")\nb = Text("B")\nb = null\na = Text("A2")')` keeps `b = null`, both `a`s and the unreachable `b`, while the same patch into any program with a statement gives `root = Root([a])\na = Text("A2")`. language.md, section 7 asks for each of those. Drop the early return and let the empty case go through the same steps.

## 17. While a later statement is still open, finished statements are treated as partial

- **Found:** 2026-10-16, by the finished-statements invariant (testing ticket 09). Reference commit `eb20708` (1.0.1).
- **Pinned by:** `hand/109-finished-while-a-later-one-is-open`, at every streaming step.
- **Status:** a candidate for the maintainer to judge.

```
root = Root([a])
a = Text("done", "loud")
note = "still being written"
```

With `Text`'s `tone` an enum of `"info"` and `"warn"`, defaulting to `"info"`.

**Reference behaviour:** once the second line is complete, `a` is `partial: false` with `tone: "info"`, the invalid `"loud"` replaced by its default. When `note = "still` arrives, `a` turns `partial: true` with `tone: "loud"`, and back again when the string closes. A finished component flickers between its corrected and its raw props while anything after it is still open.

**Why:** a result is parsed with one `partial` flag (`wasIncomplete`, the pending tail needed closing), and materializing uses it for every statement: each element is marked with it, and validation lets invalid values through while it's set. The element type's own documentation says `partial` is "True when the parser hasn't received all tokens for this node yet", and language.md, section 4, rule 1 says "Completed statements are parsed once and never change."

**Exception to the finished-statements invariant:** a change between two results whose `meta.incomplete` differs.

**Draft upstream report:**

> **A finished component turns `partial`, with its raw props, whenever a later statement is still being written.** In `createStreamParser`, the `partial` context of a result comes from whether the pending tail needed closing, and materializing applies it to every statement: completed ones are marked `partial: true` and validated leniently too. Streaming `root = Root([a])\na = Text("done", "loud")\n`, then `note = "still`, flips `a` from `{tone: "info"}` (the default, since `"loud"` isn't allowed) to `partial: true` with `{tone: "loud"}`, and back when the string closes. `ElementNode.partial` is documented as "the parser hasn't received all tokens for this node yet", and language.md, section 4 says completed statements never change. Materialize completed statements with `partial: false` whatever the tail is.

## 18. An empty program parses as incomplete

- **Found:** 2026-10-16, by the fences invariant (testing ticket 09). Reference commit `eb20708` (1.0.1).
- **Pinned by:** `hand/110-only-a-comment`.
- **Status:** a candidate for the maintainer to judge.

```
// nothing here
```

**Reference behaviour:** `parse()` gives `meta.incomplete: true` for an empty program, or one that's empty once comments and fences are stripped. A reply that's only prose with no statements, `"Sorry, that's not something I can show."`, can be incomplete too (here, through quirk 1's apostrophe), while one without an apostrophe is complete. Wrapped in a fence with prose around it, the empty program is complete.

**Why:** `parse` returns `emptyResult()` when the preprocessed text is empty, and `emptyResult`'s `incomplete` defaults to `true`, the right answer for a stream that hasn't started. A one-shot parse has all of its text.

**Exception to the fences invariant:** a program that's empty once preprocessed. **And to the chunking and stream-end invariants:** a program without statements whose results differ only in `meta.incomplete` (one-shot and the stream's end can disagree there too: `((#'\n])\n()` is incomplete one-shot and complete once streamed).

**Draft upstream report:**

> **`parse("")` says the result is incomplete.** For an empty input (or one that's empty once comments and fences are stripped), `parse` returns `emptyResult()`, whose `incomplete` defaults to `true`, as it should for a stream that hasn't started. A host that shows a loading state until `meta.incomplete` is false keeps showing it for an empty reply. Pass `false` from `parse`: `return emptyResult(false)`.

## 19. A stray character before a ternary's `?` ends a streamed statement but not a parsed one

- **Found:** 2026-10-16, by the stream-end invariant (testing ticket 09). Reference commit `eb20708` (1.0.1).
- **Pinned by:** `hand/111-stray-character-before-a-ternary`, at every streaming step and the stream's end.
- **Status:** a candidate for the maintainer to judge, on input no model is likely to write.

```
root = Root([a])
a = Text("x")
`? "y" : "z"
```

**Reference behaviour:** `parse()` reads the third line as `a`'s ternary continuing (`a = Text("x") ? "y" : "z"`): the lexer skips the backtick, so the line's first token is `?`. A stream ends `a` at the newline instead, since the next line's first character isn't `?`, and the third line is then no statement. The stream's end and the one-shot parse differ. A backslash does the same.

**Why:** the batch split looks at the next line's first token (language.md, section 1.4: "the next line's first non-whitespace token is `?`"), and `scanNewCompleted` in the stream parser at its first non-whitespace character. Section 1.4 says the two "MUST agree on these boundaries byte for byte".

**Exception to the chunking and stream-end invariants:** a line whose first token is `?` but whose first character isn't, including a `?` on a later line after a line holding only what the lexer skips (`` root=R()\n`\n? ``, found four times in a million programs by the hardening run, #19).

**Draft upstream report:**

> **The stream parser and `parse()` split statements differently before a `?` that follows a skipped character.** `scanNewCompleted` checks whether the next line's first non-whitespace *character* is `?`, while the batch split checks its first *token*, and the lexer skips characters such as a backtick or a backslash. So for `root = Root([a])\na = Text("x")\n\`? "y" : "z"`, `parse()` continues `a`'s ternary and a stream ends `a` at the newline. language.md, section 1.4 says the batch parser, the streaming parser and the merge must agree on these boundaries byte for byte. Look for the next line's first token in the stream parser too.

## 21. A statement reached through a reference cycle comes out two ways, and converting back to source keeps one

- **Found:** 2026-11-06, by the round-trip and merge-own-source invariants on a million programs against lang-core 1.1.0 (`47b9cae`; lang-core 1.1 ticket 08): 91 of the 100 programs broken on both parsers. 1.0.1's run (`eb20708`) had 1.
- **Pinned by:** `hand/142-a-reference-cycle-through-a-list` (the tree, and the source converting it back writes), and `hand/143-a-reference-cycle-written-back` (that source, parsed again).
- **Status:** a quirk, decided by the maintainer on 2026-11-07 (openui-rs #1). An upstream report is drafted for the maintainer to file (openui-rs #18).

```
root = Root([x, y])
y = Root([x])
x = [y]
```

**Reference behaviour:** `y` comes out as two different elements in one tree. Reached through `x` (the first child, `[y]`), it's `Root` with `children: []`: its own `x` closes the cycle back to `x` and is cut. Reached directly (the second child), it's `Root` with `children: [[]]`: its `x` is materialized, and the `y` inside that is cut. Converting the tree back to source writes each statement once, from the first place it meets it: `root = Root([[y], y])` and `y = Root([])`. Parsed again, both `y`s are `children: []`, so the tree changes. Merging the program's own source back into it gives the same changed tree.

**Why:** materializing memoizes a statement's result per pass, except a result that met an unresolved or cyclic reference, which "depends on the path, so it is not memoized" (`resolveRef` in `materialize.ts`). That's deliberate: where a cycle is cut depends on where it was entered. But every element keeps its `statementId`, and `jsonToOpenUI` writes one definition per statement id, so a statement that came out two ways can't be written back.

**Exception to the round-trip and merge-own-source invariants:** a program whose statements refer to each other in a cycle, and whose tree has one statement as two different elements.

**Upstream report:** drafted as a `ready-for-human` issue (openui-rs #18) for the maintainer to file.

## 22. An open ternary takes in what merging writes after it, so a second merge reads the program differently

- **Found:** 2026-11-06, by the merge-twice invariant on a million programs against lang-core 1.1.0 (`47b9cae`; lang-core 1.1 ticket 08): 7 programs, broken on both parsers.
- **Pinned by:** merge cases `hand/an-open-ternary-in-the-patch`, `hand/an-open-ternary-in-the-patch-merged-again`, `hand/an-open-ternary-in-the-program` and `hand/an-open-ternary-in-the-program-merged-again`, and `hand/144-an-open-ternary-takes-in-the-next-line`.
- **Status:** a quirk, decided by the maintainer on 2026-11-07 (openui-rs #1). It follows from 1.1's fix for quirk 6. No upstream report.

```
existing:  root = Root([a])          patch:  root = $flag
           a = Text("A")                       ? a
```

**Reference behaviour:** the first merge gives `root = $flag\n  ? a\na = Text("A")`. Parsed, `root`'s ternary is still open (a `?` without its `:`), so `a = Text("A")` is read as part of `root`, and `a` is unresolved. Merging the same patch again gives `root = $flag\n  ? a`: the second merge splits the program as the parser does, sees `a = Text("A")` as part of `root`, and replaces it with the patch's `root`. An open ternary in the existing program does the same from the other side: `s = 0 ?` merged with `$n = 0` gives `s = 0 ?\n$n = 0`, which parses as `s`'s ternary, so the second merge doesn't find `$n` and appends it again.

**Why:** a statement doesn't end at a newline while a ternary is open (language.md, section 4, rule 1). The merge writes what it keeps or appends after such a statement, as quirk 15's unfinished strings and brackets do. Since 1.1 (`47b9cae`, #1480, the fix for quirk 6), the merge splits both texts with the parser's own statement splitting, so the next merge reads the joined statement as one. Before, it split at every newline, which dropped a ternary's continuation lines instead (quirk 6).

**Exception to the merge invariants:** a program, a patch or a first merge's result with a statement whose ternary is still open: more `?` than `:` outside brackets.

# Fixed upstream

The reference no longer behaves this way, so the core doesn't either. The cases that pinned each quirk now pin the fixed behaviour.

### 6. Merging drops a ternary's continuation lines

**Fixed in 1.1.0** (`47b9cae`, #1480: "`mergeStatements` keeps multi-line statements, such as ternaries, intact"). The merge splits with the parser's own statement splitting now, so a ternary's continuation lines stay with it. The merged program is the existing one, unchanged. `hand/ternary-continuation` pins the fixed behaviour.

As recorded before the fix:

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

### 8. An id defined twice in the existing program comes out twice

**Fixed in 1.1.0** (`47b9cae`). An id defined twice keeps its first place in the merge's order, with its latest text. The example merges to `root = Stack([a])` and `a = Title("3")`, with `b` dropped as unreachable. The three cases pin the fixed behaviour.

As recorded before the fix:

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

**Exception to the merge invariants:** an existing program that defines a name twice.

**Draft upstream report:**

> **`mergeStatements` duplicates a statement that's defined twice in the existing program.** Every existing statement's id is pushed onto `order` (`merge.ts`), so an id defined twice is listed twice and printed twice with its last text. Push an id only the first time it's seen, as the patch loop already does. Repro: `mergeStatements('root = Stack([a])\na = Title("1")\na = Title("3")', 'b = Title("B")')`.

### 20. Merging reads comments as code

**Fixed in 1.1.0** (`47b9cae`). The lexer skips `//` and `#` comments itself, and the merge splits with the parser's lexer, so a quote or bracket in a comment no longer joins or swallows statements. The example merges to `root = Root([a, b])`, `a = Text("A2")` and `b = Text("B")`. `hand/a-quote-in-a-comment` pins the fixed behaviour.

As recorded before the fix:

- **Found:** 2026-10-16, by the merge invariants (testing ticket 09). Reference commit `eb20708` (1.0.1).
- **Pinned by:** merge case `hand/a-quote-in-a-comment`.
- **Status:** a candidate for the maintainer to judge.

```
existing:  root = Root([a, b])
           a = Text("A")
           b = Text("B")
patch:     // say "hi
           a = Text("A2")
```

**Reference behaviour:** the merge returns the program unchanged, so `a` stays `"A"`: the quote in the comment opens a string in the merge's view, which takes in `a = Text("A2")`, and the patch has no statements left. A bracket in a comment (`// keep b (it is fine`) joins lines the same way, and a comment line is copied into the merged program. A model that writes `// update the "Save" button` above its patch loses the patch.

**Why:** `mergeStatements` splits both texts with `splitStatementSource`, which follows brackets and quotes but doesn't skip `//` or `#` comments. Parsing strips comments first (language.md, section 1.2, "Comment stripping"), and section 7 says the merge "MUST read both texts with sections 1.2 and 1.4".

**Exception to the merge invariants:** a program or patch with a comment marker (`//` or `#`) in it.

**Draft upstream report:**

> **A quote or bracket in a comment makes `mergeStatements` lose statements.** `splitStatementSource` tracks brackets and quotes but not comments, so a patch `// say "hi\na = Text("A2")` has no statements as the merge reads it (the quote opens a string that runs to the end), and `mergeStatements('root = Root([a, b])\na = Text("A")\nb = Text("B")', patch)` returns the program unchanged. language.md, section 7 says the merge reads both texts with section 1.2, which strips comments first. Strip comments (as `preprocess` does) before splitting, or skip them while scanning.
