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

Upstream ties a call to the query that started it, tracks loading as a flag on
that query, and releases cache entries at points that follow from its
implementation. Three defects follow. A query can stay loading forever, or
hide a call that is in flight. A result can be thrown away although another
query wants it, or settle a query it was never fetched for. And the cache
grows without bound: one query cycling through keys keeps every entry it ever
left.

We do not copy this. A query's key is its evaluated tool, arguments and
dependencies; a cache entry holds one key's data and at most one call in
flight. Four rules decide everything:

1. After each command, a cache entry exists exactly while a registered query
   holds its key as its current key or its previous key (the one whose data
   it shows while loading). So there are at most two entries per query.
2. A query is loading exactly while its current entry has a call in flight.
3. A call belongs to the entry it was started for. Its success or failure is
   delivered to every query whose current key is that entry, whichever query
   started it. If no query's current key is that entry, or the entry is gone,
   the call just ends: no data, no error and no fetch history change, even if
   the same key has since been recreated with a call of its own.
4. A current entry with neither data nor a call requests one, unless it has
   failed and the query already shows an error. No query is left with
   nothing to show and nothing on its way.

A delivered success stores the result, clears each current reader's error,
records that the reader has fetched and releases its previous entry. A
delivered failure records an error for each current reader. A query can show
another query's cached data without having fetched it. A run requested while
a call is in flight is remembered for the key, and while disposed a late
success and a late failure are both ignored.

Everything else is upstream's behaviour, byte for byte. In particular a
result that arrives after its query has moved to another key is discarded, as
upstream does on purpose: a search-as-you-type query never shows a stale
intermediate result. Scripts in which a query's key only moves forward match
upstream on every observation.

The cost is on the other side of rule 1. A query that returns to a key it
left two or more changes ago fetches again, where upstream reuses the result
from the entry it leaked. That is one extra tool call, never a wrong result.

Rule 4 also decides a case found while reviewing #86 (2026-11-19). A query
edited to read its own result can move to a key that has no data while the
entry it was reading still has a call in flight. When that call delivers,
upstream leaves the query idle on the empty key, showing the old value, and
invalidating it does nothing. Here the delivery releases the query and its
call is requested, because rule 4 leaves no query with nothing to show and
nothing on its way. The first implementation matched upstream by accident:
it decided the query was waiting before moving its key, so an equal value
delivered later never woke it, and the replay's rule check reported the wait
as unexplained. Matching upstream would need an exception to rule 4 for one
shape of program. The cost is one tool call upstream does not make. Both
sides are pinned (`a-previous-key-releases-a-waiting-self-reader` in
CONFORMANCE.md).

This is a divergence on inputs a real program can produce, which is why the
glossary's definition of a divergence was widened. CONFORMANCE.md lists every
situation in which output differs, with the rule that forces it; each is
pinned with both sides' outputs, a randomised replay against upstream fails
on any difference the list does not explain, and a draft upstream report is
kept for the maintainer to file. If upstream fixes these differently, we
follow upstream and retire the divergence.
