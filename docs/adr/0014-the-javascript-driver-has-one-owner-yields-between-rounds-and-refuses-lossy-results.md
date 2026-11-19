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

The costs, as measured in the review:

- Each round after the first waits one timer turn: about 1.3 ms in Bun, 4 ms
  or more in browsers, which stretch nested timers. 200 dependent calls took
  272 ms in Bun and would take about 0.8 s in a browser.
- A write made while a call is pending, or while a completion's turn is
  queued, dispatches one task later instead of at once.

The yield uses `setTimeout(0)`. A faster task source (`MessageChannel`, or
`setImmediate` where it exists) keeps the rule and lowers the cost; #109
tracks it. Replacing the yield with a microtask would break the rule, because
microtasks run before timers and input.

## A result that JSON would change is an error

Upstream keeps whatever JavaScript value a provider returns. The driver passes
the result into WebAssembly as JSON, so a value JSON cannot hold must become
something. The rule: the call ends with a readable `tool-error` when the value
would be lost or would change kind, and `JSON.stringify`'s behaviour is
followed where a JavaScript developer already expects it.

- **Error:** a function, a non-finite number (`NaN`, `Infinity`), a BigInt, a
  symbol as a value, `Map`, `Set`, a boxed non-finite number, an invalid
  `Date`, and a result that is not representable at all (a cycle, a throwing
  `toJSON`).
- **Follow `JSON.stringify`:** a property whose value is `undefined` is
  dropped, a valid `Date` becomes its ISO string, an `undefined` array item
  becomes `null`.

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

As of this record, functions, non-finite numbers and BigInt give errors and
are pinned in CONFORMANCE.md. `Map`, `Set`, nested symbols, boxed non-finite
numbers and invalid dates still pass through `JSON.stringify` silently, and
the three accepted conversions have no conformance rows; #109 carries that
work. Each case that differs from upstream is a divergence with both sides
pinned.
