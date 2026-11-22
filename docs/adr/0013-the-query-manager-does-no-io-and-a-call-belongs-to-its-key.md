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

A waiting query requests nothing, and its wait has a source that can still
deliver, with one exception copied from upstream (decided by the maintainer
on 2026-11-21, #188): when the parser lists a statement that is not a query
in a query's wait list (a Query wrapped in a call the library does not
define but whose name is an `Object.prototype` member, such as
`@toString(...)`, which the parser counts as a reserved call), nothing can deliver to it, so the query
shows its default and never calls, exactly as lang-core 1.1.0 does. The
cache audit and the query fuzz target accept that wait; a wait on a query
statement still needs a source that explains it (pinned by
`a-reader-of-a-statement-that-is-not-a-query-keeps-its-default` in
CONFORMANCE.md).

(2026-11-23, #210. Rule 4 accepts a wait on a missing source whose query statements are all incomplete if it never registered in this program. A source registered while complete keeps its registration when it becomes incomplete. A registered reader that becomes incomplete keeps its last wait even after its source delivers.)

(2026-11-23: since #223, a suspended reader whose statement becomes incomplete keeps its suspension, error and propagation limit diagnostic and requests no replacement call until a successful registration evaluates it again or removes it.)

Rules 1 and 3 decide a second case, found by the query fuzz target after
#86 merged and confirmed by the maintainer on 2026-11-20. When one command
moves a reader through several keys (`c` reads `b`, `b` reads `a`, every
source a cache hit), upstream calls the tool for each key the reader passes
through: three calls in the pinned scenario, of which only the last result
is used. Here requests are collected while readers are re-evaluated and a
call is issued once the command's cascade has finished, for each key that is
still some query's current key. A key a reader only passed through has no
entry after the command (rule 1), so it has no call (rule 3). The first
implementation issued a call at each move and then retired it within the
same command, which handed drivers calls nobody wanted. The cost is none to
the app: it makes fewer requests and sees the same final result. Both sides
are pinned (`a-reader-only-dispatches-keys-still-current-after-the-cascade`
in CONFORMANCE.md).

Rule 3 and the final-key dispatch meet in a third case, found while
reviewing the dispatch fix (#113) and decided by the maintainer on
2026-11-20 (#119). A run asked for while a call is in flight is remembered
for the key. When that call is delivered, its readers can move the key's
only holder to another key within the same command, so at the end of the
cascade the key has no current holder and the run is not started. The
first implementation then forgot it, which is what the documentation of
Invalidate said ("forgotten if none is"), and a query that later returned
to the key showed the delivered data as settled, although a run had been
asked for after the call that fetched it. Now the entry is marked stale
instead: it is still held as the query's previous key (rule 1), and a
query that makes it current again starts the run it owes, showing the
delivered data while loading. The mark is cleared by any call the entry
starts, and the entry is released like any other once no query holds it.
Rule 4 grows one clause with it: a ready query on a stale current entry
requests a call, whatever its data or error. The cache audit checks that a
stale entry has no call in flight and, after a registration, is the current
entry of no query that is ready (neither waiting nor suspended). A run remembered while the holder moved away
before the delivery (by an edit) is still forgotten with the undelivered
call, as rule 3 says; only a delivered call owes its run. Lang-core starts
the run at the delivery, while the query is still on the key, and leaves it
in flight on a key nobody shows. Both sides are pinned
(`a-remembered-run-marks-its-key-stale` in CONFORMANCE.md).

The same review asked whether keeping the first request for a key can call
a key a reader passed through with another query's tool name or arguments.
It can, when two queries' keys collide and their raw values differ, and
lang-core does the same: it fetches at the first request. The call belongs
to the key, and colliding keys share a call as upstream's do, so this is
kept (`a-colliding-key-keeps-the-first-request-across-a-cascade`).

Two decisions of #119 sit beside the manager. Retry runs every failed
query, including those whose errors the snapshot holds only in the compact
error (#90); each retried result passes the same limits, so a result that
was refused is refused again. And a program can be unregistered: the
manager's registrations belong to another program (the session compares the
identity of the parse result last registered), and registering this one
was refused by a limit. A refused command leaves the manager unchanged, so
everything it holds is another program's, even where a statement id is the
same. While a program is unregistered nothing of the other program is
drawn, read by an action or a resumed plan, called or refreshed. Each query
draws and reads its own default and each mutation is idle. The query
snapshot beside a draw lists nothing loading, fetched or failed, and the
registration's limit is the draw's query error. `@Run` of a query and Retry
start no call; the other program's refresh timers do not fire, and no
refresh deadline is reported. A mutation step fails for the registration's
limit before any call, as a step does at the pending-plan limit: the
action's failure names the step, `sizeLimited` is set and the rest of the
plan does not run. Calls the registrations start meanwhile, such as a late
result's readers, stay with the session. A registration of the program they
belong to hands over those still pending. A registration of another program
hands over a held query call only if its key is current for one of the new
program's queries (whose registration requested no second call for it); a
held query call whose key only a previous slot holds is ended by the manager
without delivery, as rule 3 ends it on delivery, so no receipt is
outstanding for a key that is not current and nothing the old program's
readers wanted reaches the driver (found by the second review: a refetch's
late result woke an old reader whose call went out under the new program).
Held mutation calls are handed over in either case: a user clicked them
while their program was drawn, and the same call goes out today when a
driver drains late without any refusal, so that question belongs to its
own follow-up. Going back to the program the registrations belong to draws
its own data again.
The review of the first version found each of these reading or calling the
old program: an action copied its result into state, a resumed plan read
it, `@Run`, Retry and timers called its tools, and the new program's button
fired the old program's mutation tool, a wrong side effect in the host.
Lang-core has no limits and registers every program; the differences are
pinned in CONFORMANCE.md.

This is a divergence on inputs a real program can produce, which is why the
glossary's definition of a divergence was widened. CONFORMANCE.md lists every
situation in which output differs, with the rule that forces it; each is
pinned with both sides' outputs, a randomised replay against upstream fails
on any difference the list does not explain, and a draft upstream report is
kept for the maintainer to file. If upstream fixes these differently, we
follow upstream and retire the divergence.

## A step that cannot run

Two states stop the manager from calling anything a plan asks for: queries
are paused (the manager is disposed until resume) and the program is
unregistered. Until #117 the four cases disagreed. A mutation step while
unregistered failed for the limit; a mutation step while paused, or a plan
whose mutation call was in flight at the pause, vanished with no call, error,
host event or anything after resume; a query `@Run` in either state started
no call and the plan went on. The maintainer decided on 2026-11-20 (#115,
decision 9) that a dropped action is reported to the app as a failure.

One rule now decides all four: a step that cannot run reports a failure
exactly when it ends its plan. A mutation step ends it, as upstream's does:
the renderer's loop returns when `fireMutation` returns false, which it does
at once while disposed and, for a call in flight at the dispose, when the
call settles. Later steps may read the mutation's result, so they cannot run
without it. The session reports that end as the action's failure: paused, of
the new kind `QueriesPaused`, at the mutation step, in the click's own output
or, for a call in flight, as a segment of the next drain; unregistered, of
the registration's limit, as before. Exactly one failure per dropped plan,
and nothing of it runs after resume. A query step does not end its plan:
upstream's `invalidate` returns without fetching while disposed and the loop
goes on to the next step. So a query `@Run` that cannot fetch, paused or
unregistered, is skipped silently and the plan carries on, matching upstream
where upstream has the state. Reporting it would mean either a failure for a
plan that went on to run, which the action failure cannot express (it names
the step that stopped the plan), or stopping the plan where upstream does
not. Both sides are pinned for pause (`paused-actions` in CONFORMANCE.md);
lang-core has no unregistered state.

A failure for a call in flight waits in the session until the next drain,
which the drivers take right after pausing. It keeps the continuation slot
its plan held until then, so at most `maxContinuations` wait at once and a
click beyond that fails at its step with the pending-plan limit. Where a
program is both paused and unregistered the limit is reported, once. The
reasons are fixed texts. Detaching the provider and disposing the session
still end plans without a report: neither leaves a program that could go on.
After a resume whose registration of the program the registrations belong to
is refused by a limit, the session asks the query manager, once, for the tool
calls of the last accepted registration, with that registration's tool names
and arguments (#142): a refused registration leaves the last accepted one as
what is drawn, exactly as when the same refusal happens without a pause, and
nothing else would ask again for the calls the pause released.
2026-11-22 (#187). A refused registration reports its limit or evaluation error beside the draw. A re-read keeps held data when its parsed registration inputs match, including state defaults. A mutation requested before replacement keeps its original tool and arguments until dispatch. The session defensively reports an evicted provider-run action once at its mutation step.
