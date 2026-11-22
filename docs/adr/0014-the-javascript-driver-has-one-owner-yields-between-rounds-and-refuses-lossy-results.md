---
status: accepted
---

# The JavaScript driver has one owner, yields between rounds and refuses lossy results

The WebAssembly package's tool provider driver (#93, `setToolProvider`) runs an
app's tool provider for a session on top of the manual interface
(`attachToolProviderAt`, `takeQueryUpdate`, `completeToolCall`). Three of its
rules are ours, not upstream's. Each was decided on 2026-11-19 after an
independent review of #93 found a defect the rule removes. This record keeps
what was observed and what was rejected, so that a later change starts from
the same facts.

ADR 0013 still binds the driver: the query manager decides what is called and
when, and a driver adds no timeout, retry or scheduling rule of its own.

## One owner per session

A session's tool calls are run by the manual interface or by the driver, never
both. `setToolProvider` is refused with `RefusedWrite` while a manual
attachment exists. The manual calls are refused while the driver is attached.
Detaching allows either next.

What was observed: an app attached by hand, took a wanted call, then called
`setToolProvider`. The driver refused the manual `completeToolCall` for that
receipt and never dispatched the call itself. The query stayed loading, Retry
did nothing, and only a detach ended it.

Rejected: letting the driver take over and end the outstanding manual calls.
The driver never saw those calls start. It would have to invent an outcome for
each, a failure the provider never reported, or run them a second time. A
refusal changes nothing and tells the app what to do.

The cost is that an app moving from manual calls to the driver must detach
first, which drops receipts still outstanding.

## A turn of the event loop between rounds, and no cap

A dispatch round is one pass of: ask the session which calls it wants, run the
provider for each, hand the results back. Every round that follows a
completion starts in a later task of the host's event loop. The number of
rounds is not capped.

What was observed: this provider writes to its own session after an `await`,
so every result makes the session want another call.

```js
read: async ({ n }) => { await null; s.set('$n', String(n + 1)); return n; }
```

Before the rule the driver ran 20,001 calls in 306 ms before a zero-delay
timer got a turn. With no bound it held a core at 100% and grew past 1 GB
until it was killed after about two and a half minutes. Yielding only during
a dispatch did not help, because the write happened after the `await`. With
the rule, one call runs before the timer for every provider shape tried
(async, `.then`, synchronous, a result getter, `toJSON`, a thenable, an MCP
client). 20,000 rounds interleave with 19,999 timer turns, and resident
memory levels off near 105 MB.

Rejected: a cap on rounds that ends a runaway provider with an error. A cap is
a scheduling rule of our own, which ADR 0013 avoids. An honest chain longer
than the cap would fail, and a runaway provider would still freeze the page
until it reached the cap. The app owns its provider and can stop it; the
driver's duty is that the page can still take the click that does so.

The costs, as measured in the review (with a zero-delay timer as the yield):

- Each round after the first waited one timer turn: about 1.3 ms in Bun,
  4 ms or more in browsers, which stretch nested timers. 200 dependent calls
  took 272 ms in Bun and would take about 0.8 s in a browser.
- A write made while a call is pending, or while a completion's turn is
  queued, dispatches one task later instead of at once.

### The task source (#109)

The yield is still a task, but no longer a timer. The driver reads the host
at each yield and takes, in order:

- **`setImmediate` where it exists (Bun, Node), posted twice.** A single
  immediate posted from a timer callback runs in the same loop iteration,
  before the next timer pass. That was observed: a timer due before the
  dispatch saw two calls. The second hop always follows a timer pass, so a
  due timer runs before the next round wherever the round was posted from.
- **One `MessageChannel` message per turn otherwise (browsers).** The port
  is unreferenced where the host allows, so an idle driver never keeps a
  server runtime alive.
- **A zero-delay timer** when neither exists.

### When a task source refuses

A host can throw from `setImmediate`, `postMessage` or `setTimeout` (a
sandbox, a shimmed global, a torn-down worker). Two defects were found by
review and reproduced: a throw from `setImmediate`'s second hop escaped
uncaught and left the turn reserved, so the driver never drained again; and
with every source refusing, releasing the turn and reporting it let an
`onError` that writes to the session drain at once, so the next completion
ran as a microtask, was refused again, and so on: 100,000 calls ran in Node
before a zero-delay timer scheduled first. Now:

- **A refusing source falls back to a timer**, from either hop. A server
  runtime does not fall back to its `MessageChannel`, which in Bun delivers
  ahead of due timers. One refusal no longer stalls a dependent chain.
- **When every source refuses**, the driver reports the error to `onError`
  once (again only after a task has run), then releases the turn. The turn
  stays reserved while `onError` runs, so a write it makes, or any other
  drain during the report, does not dispatch at once. With no way to queue
  a task the driver cannot continue on its own, and it must not continue
  synchronously: that is the loop that held the event loop. The next session
  operation drains once, unless provider code has paused or detached (the
  cancelled mark below): the mark is cleared only by a driver's turn, which
  a refusing host never runs. So on a host that refuses permanently, a pause
  or detach made from provider code leaves the driver waiting until a task
  source works again; then the next operation posts a turn and recovers it.
  A third review showed why the mark cannot simply be cleared on refusal:
  with `setImmediate` and `MessageChannel` absent and `setTimeout` throwing,
  `read: async () => { await null; s.pauseQueries(); s.resumeQueries(); }`
  ran 20,000 calls in 308 ms before the first task.

Tests pin both: a refused first or second hop finishes a dependent chain
with no error, and with every source refusing a timer scheduled before the
dispatch runs after at most two calls, `onError` hears one report, and the
next operation dispatches once. A provider that pauses and resumes on such
a host is held to at most three calls before a due timer, and finishes its
chain once the timer works again.

On Bun and Node the turn comes from `setImmediate`, so test fakes that
replace only `setTimeout` no longer govern it; fake `setImmediate` too.

A completion reserves its turn before reading the provider's result, as
before, so a write from a getter or `toJSON` cannot dispatch at once; it
now posts the turn after the read. A timer that the getter schedules is
therefore queued ahead of the round, and runs first on every host,
including the timer fallback, where it used to run one round late.

### Session operations from provider code or a callback (#109)

A second review found that the reservation was not enough: provider code
or a callback can cancel the driver. This was reproduced:

```js
read: () => ({ toJSON() { s.pauseQueries(); s.resumeQueries(); return 1; } })
```

The pause cleared the completion's reservation, and the resume's drain
dispatched at once, inside the serialization. A due timer first ran after
5,000 calls. Detaching and reattaching from `onChange` recursed until the
stack overflowed, and left the session refusing every later call as
already in use. The rule is now kept per session, not per driver, so a
replaced driver cannot escape it:

- **Busy.** While a drain or a completion is on the stack (which covers
  `toJSON`, getters, `onChange`, `onAction` and `onError`), any drain the
  session is asked for waits for a later task. A detached driver's final
  drain is skipped instead; the session keeps the change for the next drain.
- **Cancelled.** Pausing, detaching or freeing while calls are still running,
  while busy, or while a turn is reserved marks the session. Until the
  current driver's turn runs, drains wait for a later task, since the
  running calls' pending count no longer shows that provider code may be on
  the stack. This covers a thenable's `then` and code in a microtask after
  a completion, where the reserved turn was the only thing holding the next
  round. A detached driver's own final drain does not clear the mark.

The test runs pause and resume, detach and reattach, and a provider
replacement from `toJSON`, a getter, a thenable's `then` and `onChange`;
a timer due before the dispatch runs after at most three calls in each case.

A third review found three ways the first version of this rule still
dispatched without bound, each followed by an evaluate or a write so that
the session wanted the call (a due timer first ran after 5,000 calls):
detaching and reattaching after an `await` (the detached driver's final
drain counted as a turn and cleared the mark); the same from a microtask
several steps after a synchronous return (nothing was pending or busy,
only the completion's reserved turn, which the cancel dropped); and the
refusing host above. A second test now runs pause and resume, detach and
reattach, and pause then detach and reattach, each followed by an evaluate
and by a write, from `toJSON`, a getter, a `then`, `onChange`, after an
`await` and from a deep microtask after return. Each holds a due timer to at
most three calls, and each chain then finishes its 50 calls once real time
passes, or, where pausing after a delivery leaves nothing to fetch, is idle
and dispatches a later write at once.

A busy count cannot be above zero at the start of a real host task, so a
turn could reset it. That was not done: a fake timer can run a turn's
callback synchronously from inside provider code, where resetting the count
would hide a real reentry.

The cost is a third bullet to the two above: pausing or detaching while a
call is pending makes the next dispatch wait one task, even when the app
does it from a click. That is indistinguishable from the same calls made
inside a provider's `then`.

What was measured:

- 200 dependent calls in Bun: 261–297 ms with the timer, 6–18 ms with the
  double immediate (the test bounds every provider shape under 100 ms).
- In the Chromium the desktop app embeds (version 156), 200 rounds took
  940–952 ms with a timer and 2.8–8.7 ms with a `MessageChannel`. A timer
  made due before each round ran between rounds, one round per timer turn
  after the first.
- For every provider shape listed above, on the `setImmediate` and timer
  hosts, a timer made due before each round sees exactly one more call per
  round; on every host a microtask chain started in a call never sees the
  next call.

Rejected:

- **`MessageChannel` in Bun.** Bun delivered all 200 messages before a due
  zero-delay timer ran. It is used only where `setImmediate` is absent; in
  Bun the test exercises it for speed and task boundaries only.
- **A single `setImmediate`.** See above: one extra round before a due
  timer when the round is posted from a timer callback.

A timer that is not yet due when a round yields (a fresh zero-delay timer is
due about 1 ms later in Bun and Node) now waits for several fast rounds
instead of one slow one. That is the point of the change: the property kept
is that the event loop is never held, not that a timer which has not fired
yet sees one call. The cost that remains is the second bullet above.

## A result that JSON would change is an error

Upstream keeps whatever JavaScript value a provider returns. The driver passes
the result into WebAssembly as JSON, so a value JSON cannot hold must become
something. The rule: the call ends with a readable `tool-error` when the value
would be lost or would change kind, and `JSON.stringify`'s behaviour is
followed where a JavaScript developer already expects it.

- **Error:** a function, a non-finite number (`NaN`, `Infinity`), a BigInt, a
  symbol as a value, `Map`, `Set`, a boxed non-finite number, an invalid
  `Date`, a typed array, `ArrayBuffer` or `DataView`, and a result that is
  not representable at all (a cycle, a throwing `toJSON`).
- **Follow `JSON.stringify`:** a property whose value is `undefined` is
  dropped, a valid `Date` becomes its ISO string, an `undefined` array item
  and a hole in a sparse array become `null`, and a symbol-keyed property is
  dropped. An object with its own `toJSON` (a `Map` or an invalid `Date`
  included) follows it: the author chose that conversion. A finite boxed
  number, string or boolean becomes its primitive.

The same rule applies to a function-map provider's result and to an MCP
client's `structuredContent`.

What was observed: the first version followed `JSON.stringify` throughout, so
`{a: fn}` became `{}` and `NaN` became `null` with no error and no documented
difference. A program reading that result drew a wrong value that nothing
reported.

Rejected, on both sides:

- **Follow `JSON.stringify` for everything.** A dropped function or a `null`
  where a number was is a silent wrong value. A `Map` becoming `{}` loses
  every entry.
- **Refuse anything that is not plain JSON.** Results with optional
  properties or `Date` fields are what real providers return, and
  `JSON.stringify` is what their authors expect to happen to them.

### Typed arrays and symbol keys (#109)

The ticket left these two open. Both were decided by asking what an OpenUI
program could read from the value upstream keeps, since lang-core 1.1.0
stores the provider's value as it is (pinned in
`conformance/golden/tool-provider-non-json.jsonl`).

- **A typed array is an error.** `JSON.stringify(new Uint8Array([1, 2]))` is
  `{"0":1,"1":2}`: the value stops being a sequence. Upstream a program reads
  the typed array's `length` (`Member` reads `obj[field]` for anything that is
  not an array, `runtime/evaluator.ts`), and after the conversion that reads
  as null. That is a change of kind. Converting it to an array instead would
  be a conversion no JavaScript developer expects from `JSON.stringify`, so
  the rule's error applies. An `ArrayBuffer` and a `DataView` serialize as
  `{}`, losing every byte, and fail for the same reason.
- **A symbol key is dropped, as `JSON.stringify` drops it.** No OpenUI
  program can name a symbol: member and index access take a name or a
  string, and every lang-core built-in that walks an object
  (`Object.keys`, `Object.entries`, `for…in`) sees string keys only. Upstream
  keeps the property, but only the app's own React components could read it,
  and the bindings never hand those components a JavaScript object. Nothing
  a program or a binding's output can observe is lost, so the error would
  refuse results that libraries decorate with symbols for no gain.

### What the driver does not see

The check runs inside `JSON.stringify`'s replacer, so it adds no traversal.
Property values are read once, by `JSON.stringify`; the driver inspects
descriptors (which runs a Proxy's `getOwnPropertyDescriptor` trap: for a
`null` value's holder and for each object's own `length`) and reads a date's
`toJSON` once more when it decides whether a `null` came from that date. A date reaches the replacer already
converted, so the driver looks at the holder's data property: a branded
date whose own `Date.prototype.toJSON` produced `null` is refused, whether
its time is invalid or a replaced `valueOf` gave `NaN`. A branded date that
reaches the replacer unconverted (its `toJSON` is not callable, so
`JSON.stringify` would write `{}`) is refused too, and so is a `DataView`
over a detached buffer (its `byteLength` brand throws; `ArrayBuffer.isView`
still recognizes it without running application code). These gaps remain,
each documented rather than closed:

- A getter that returns an invalid date still becomes `null`. Reading the
  getter again would run application code twice.
- A built-in is recognized by its prototype chain, confirmed by a brand
  check. A `Map`, `Set`, `Number` or `Boolean` object from another realm, or
  one whose prototype was replaced, serializes as `JSON.stringify` makes it
  (`{}`, or for a number the result of the replaced `valueOf`, which can be
  `null`). A brand check on every object would throw for each ordinary class
  instance, about 0.3 µs each in Bun and 3 µs in Node, and nothing marks
  these objects cheaply. A boxed string does have a mark, its own `length`,
  so it is handled (below).
- Other built-ins that `JSON.stringify` empties (`WeakMap`, `WeakSet`,
  `WeakRef`, `Promise`, `RegExp`, an `Error`'s message) still pass through
  it. They are not in the ticket's list; extending the table is one line
  each if a later decision wants them.

### Text the output gains is charged

The transport budget bounds the text a result can make before Rust admits
it. Every string the replacer sees is charged, but `JSON.stringify` can
write text the replacer never sees as a string:

- **A boxed string.** `JSON.stringify` writes `ToString` of it, which calls
  its prototype's `toString`. It is the only object `JSON.stringify` turns
  into text by calling application code, so it is recognized first, before
  any classification, whatever its prototype: a `String` object's own
  `length` is always a read-only, non-enumerable, non-configurable data
  property, and only an object with such a property pays the throwing
  `String.prototype.valueOf` brand. A boxed string is then written as its
  own value and charged; its `toString` never runs. Two reviews found
  holes in earlier versions: a boxed string with a replaced prototype was
  not unboxed (64 copies of a 4 Mi string grew Bun by 985 MB, 256 by
  4.8 GB), and then one whose replaced chain reached a typed array's
  prototype passed as an unbranded typed array (64 copies: 976 MB). With
  the string check first, N = 1, 64 and 256 copies of that repro publish
  one-character strings in under 3 ms with under 5 MB of growth. A test
  builds a boxed string over every recognized prototype, plain, through a
  Proxy and with its own `toJSON`, also handed back by a date's
  `toISOString`, and asserts the string's own value or a driver error each
  time. Checking only objects with such a `length` costs 200,000
  `{length}` objects about 115 ms in Bun, against 226 ms when every own
  `length` paid the throw.
- **Raw JSON** (`JSON.rawJSON`) is written as its text, which is charged.
- **`toJSON`, including `Date`'s.** Whatever it returns (a date's
  `toISOString` result included) goes back through the replacer, so a string
  is charged and a boxed string is unboxed as above.

Every entry of the brand table was audited for an object that passes
through unclassified: each brand throws for any other object (the
typed-array tag getter returns `undefined` instead, so its brand now throws
when the tag is missing), a branded built-in is refused or unboxed, and an
object that fails its brand is serialized as an ordinary object, which can
no longer be a boxed string.
- A `Number` or `Boolean` object, whatever its prototype, can only write a
  number or a boolean; a `BigInt` object throws. Numbers, punctuation and
  string escapes make the output a bounded multiple of the charge (at most
  about 25 characters per charged visit, 6 per charged string character),
  which the session's own admission then measures exactly.

As of #109 every error and accepted conversion above has a conformance row
with both sides pinned. Each case that differs from upstream is a divergence.

## Freeing the session delivers what it produced, then calls nothing

#117 made the driver report an action a pause drops, and three reviews of
its delivery settled this rule (the maintainer's narrowed version,
2026-11-21):

> After the app's `free()` or `dispose()` returns, the driver calls none of
> the app's callbacks (`onError`, `onAction`, `onChange`), invokes no tool
> provider and delivers no result into the session, for the session's
> current driver, a closing driver and any driver detached earlier. Every
> action segment the session produced before that is delivered exactly
> once, in order, before the free returns.

How it holds:

- A drain holds the update it took, with a cursor that each delivery
  advances before its callback runs. Free and dispose first deliver the rest
  of that update, then take what the session still holds (a pause's failures
  whose drain waits for a task), again while an update brings segments, so
  one a callback produces meanwhile is not lost.
- That repetition is bounded at three takes. One pause leaves at most
  `maxContinuations` failures, and a callback can produce more only by
  resuming, clicking and pausing again, so one extra round is what a
  reasonable app needs and a third is generous. An app that does it on every
  delivery made `free()` deliver 20,001 segments in 1.2 s in review, and
  without a bound it never returns. Past the bound the driver stops taking,
  reports one `LimitExceeded` through `onError` saying so, and the free goes
  on; what is left goes with the session.
- The freed or disposed session joins a set every driver checks: `notify`,
  a callback's failure handler and `report` return at once, so a driver
  detached earlier, a closing driver and the current one all stop. A
  driver attached inside a closing drain's callback and the closing driver
  both end.
- Detach delivers the same way while the driver is still attached,
  reporting the evaluation error and reader-round cut of each update it
  takes as a drain would. The detached driver's last drain then delivers
  what it takes, as it always did; it is the session's closing driver
  meanwhile, so a free made inside one of its callbacks finishes it.
  Nothing is left in the session for a later owner.
- A tool provider is invoked only by a drain of an attached driver, and a
  result is delivered only through its receipt, which free revokes.

A free made inside a callback calls `onError` and `onAction` again before
it returns: the app is re-entered inside its own callback. We accept that:
the segments are the session's last words, the re-entry is synchronous and
bounded, it happens only inside a call the app made to end the session, and
callbacks already tolerate session calls made from them. Rejected:
delivering the rest from a later task (it breaks the rule), dropping it (a
dropped action reported zero times), and handing it back some other way (a
new channel for one case; `free()` is also `Symbol.dispose`, whose result is
ignored). A pause inside a callback no longer stops the delivery of segments
the drain already took: they were produced before it, and the plan they
report really ran. (A resume never stopped it.)

### The limit, and what was tried

The driver does not promise that JavaScript's own promise machinery leaves
alone values the app created. A job already queued may still read or run a
thenable's `then`, a promise subclass's constructor or `Symbol.species`, or
a getter on such a value after the free. Cases from the third review:

- A released query receipt whose provider thenable later resolves with
  another thenable: that thenable's `then` is read and called after free.
- A free made inside the provider, or inside a callback, after which the
  value it returned has its `then` read by `Promise.resolve`'s job.
- A `Promise` subclass, or a native promise with a patched `constructor`:
  its constructor and `Symbol.species` run from a native job after free.

None of these can call a callback, invoke a provider or deliver: the
receipt is revoked and every driver has stopped. Closing them means
re-implementing promise resolution inside the driver. The second fix round
did that: a hand-written adoption of provider thenables that called `then`
from its own job only while the receipt could deliver, and a callback's
returned thenable having its `then` called at once. The third review ran 60
provider shapes against both and found it equivalent for ordinary shapes,
but not for a live session in several: a native promise with its own
`then`, a patched constructor or a subclass delivered two microtasks later,
read `then` twice and in one case delivered a different value; it used the
host's `queueMicrotask`, which Jest and sinon fake by default, moving
delivery behind a timer; a callback's thenable resolving with a rejecting
thenable was no longer reported; and a throwing `then` reached `onError`
synchronously. That trade, live behaviour for values touched after the end,
was withdrawn. The provider-result path and a callback's return value are
followed with `Promise.resolve`, exactly as before #117, and tests pin that
behaviour so a change to it is deliberate.

(2026-11-22: since #144, the driver reads each result conversion and property once per JSON position, including a getter returning an invalid Date, and inspects each own descriptor at most once. A busy host turn waits for a native microtask checkpoint before repairing the count and posting another task. Each action segment keeps its callbacks through delivery. A stop takes at most one update plus one per continuation slot, shared with nested stops, and refuses guarded reentry before flushing.)
