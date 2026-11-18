---
status: accepted
---

# The query manager does no I/O, and a call belongs to its key

The runtime's query manager (#79) never makes a network call and never reads
the time. It is a state machine. After each command it reports the tool calls
it wants performed and when the next refresh is due. The embedder reports each
call's outcome and supplies the current time. A thin driver in each binding's
language turns that into one tool provider function for the app.

Three alternatives were rejected:

- **A blocking callback**, as host functions use (ADR 0009). A tool call takes
  network time; the session would be frozen for all of it, with no input and
  no redraw while data loads.
- **Async Rust** (an async trait and a runtime). It adds a large dependency to
  crates that have none, needs a different runtime on WebAssembly, and puts
  networking and credentials inside our code.
- **Leaving it to each app**, as before. Every embedder would schedule queries
  differently and nothing could be compared with upstream.

The cost is a small driver per language. In return behaviour is identical on
every platform, and every scenario can be replayed against upstream's own
`createQueryManager` one step at a time with a scripted provider and a fake
clock.

The manager has no timeouts, retries, backoff or cancellation. Upstream has
none, and a timeout would need a clock rule of our own. A call that never
finishes leaves its query loading. Drivers cancel outstanding calls when the
session is disposed; an app that wants a timeout applies it in its provider.

## A call belongs to its key

Upstream ties a call to the query that started it. When two queries share a
key (the same tool, arguments and dependencies), the second is not shown as
loading; if the first changes its key mid-call, a success is thrown away
although the second still wants it, while a failure is delivered; a run
requested through the second is lost; and after a dispose a late failure is
recorded while a late success is ignored.

We do not copy this. A call belongs to its key:

- its success or failure goes to every query that currently wants the key, and
  is dropped if none does;
- every query whose key has a call in flight is loading;
- a run requested while the call is in flight is remembered for the key;
- while disposed, a late success and a late failure are both ignored.

Where no two queries share a key and nothing is disposed mid-call, output is
byte-identical to upstream. This is a divergence on inputs a real program can
produce, which is why the glossary's definition of a divergence was widened.
It is pinned with both sides' outputs, and a draft upstream report is kept for
the maintainer to file. If upstream fixes it differently, we follow upstream
and retire the divergence.
