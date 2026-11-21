# Swift and Kotlin runtime sessions

The [generated application signatures](runtime-binding-api.md) list constructors,
methods, records and typed errors. Create RuntimeSession for a ComponentLibrary;
its original schema supplies action definitions. Supply the prompt spec's library
object or full wrapper for BindingProps. A library made from a
[library definition](library-definition.md) gives that library object as
`promptSpec()`. undecidedBindings reports ambiguous
pairs; markBinding resolves them. registerFunction supplies declared library
functions; unregistered functions still return null. See the callback contract below.

**Kotlin streaming must preserve UTF-16 pairs.** Generated UniFFI
FfiConverterString.toUtf8 uses Charsets.UTF_8.newEncoder(),
onMalformedInput(CodingErrorAction.REPORT), and encode(CharBuffer). A Kotlin
String containing a lone surrogate throws **java.nio.charset.MalformedInputException**
(a CharacterCodingException), before Rust receives the call. This is not an
OpenUiException. It applies to every string argument, including StreamSession.push;
a chunk boundary splitting a valid surrogate pair will therefore throw.

Buffer a trailing high surrogate until the next chunk. Join it to that chunk
before calling push; retain another trailing high surrogate if necessary.
Reject an unmatched high surrogate at end of stream. Other malformed UTF-16
should still throw rather than silently change the text. The demo provides the
small [Utf16StreamBuffer caller adapter](../android-demo/app/src/main/java/dev/openui/demo/Utf16StreamBuffer.kt):

```kotlin
val buffered = Utf16StreamBuffer(StreamSession(library, null))
buffered.push(chunk) // ParseOutput; current result when holding the entire chunk
buffered.finish()    // throws MalformedInputException for an unmatched final unit
buffered.discardPending() // explicitly discard a held unit after that refusal
```

Its Android test checks direct push rejection, a split emoji pair, an empty
intervening chunk, a leading lone low surrogate, end-of-stream rejection, explicit
pending-unit recovery and unchanged state after failed
lowering. The demo's chunksOf already divides complete source by code points.
For intentionally preserved lone units, send ASCII JSON escapes, for example
`session.set("\$text", "\"\\ud800\"")`; the core's JSON reader preserves them.
Swift strings repair malformed native UTF-16, while the same JSON-escape route
preserves lone units there. Native tests require their platform toolchains.

Every parse/parseWithLimits and StreamSession output carries an opaque
ParsedProgram containing the actual Arc<ParseResult>. setProgram shares it;
editing ParseOutput.json/flags cannot alter the program. Call setProgram and
initialize after each desired push. Initialization applies declarations and
optional persisted-object JSON with Store's pristine rules, returning changed
and per-name refusals. State names include `$`. Optional JSON strings distinguish
undefined (absent) from null (`"null"`); snapshots omit undefined. Form writes
report changed/saveRequested; initializeField applies the streaming/default guard.

StreamSession caches its current output and bounded source text. Repeated result,
empty push/pushAll, pushes ignored past the input limit, repeated finish and
a known uncut no-op set share the same
ParsedProgram; each returned JSON string remains owned by its record. Real pushes
and resets retain the actual new parse result. JSON equality is never used to
merge programs: it can hide container identities relevant to initialization.
There is one cached result/JSON, not a history. Source-text caching stops at the
parser's program-size bound. Serialization and parser work for actual updates
remain the core's; retaining unchanged outputs adds program reference counts.

Evaluate returns exactly TreeResult::to_json (`{root,errors}`), separate cut flags
and an EvaluationHandle. Runtime evaluation determines the cut flags and draws
use only runtime limits.
Encoding adds no independent size, work or depth quota. `maxJsonBytes` bounds
host input, not tree JSON or its envelope. The worker
retains just the latest evaluated root; it does no plan indexing, node-budget
walk or per-plan allocation. **maxPlans has been removed.** actionAt walks the
root lazily in O(path length), borrowing props rather than copying them. For
example `["props","children","0","props","onClick"]` follows JSON from root;
array indices use canonical decimal strings. This works for nested elements,
objects, arrays, Each output and plans read from state. Action/evaluation handles
retain no root or old program, only identity/generation/path. A successful draw
or accepted program replacement releases the old root and stales its handles.
Wrong-session objects throw ForeignHandle. A refused program preserves the
preceding evaluation. No IDs or capability fields enter tree JSON.

SetReferenceValues takes an object keyed by query/mutation id. Preparation happens
once and is reused across calls, with independent limits matching Rust's
PreparedReferenceValues::new (depth 32768 and default work/size ceilings).
The latest root retains the data actually rendered, as a bounded runtime result;
replacing it frees that root. Neither pending runs nor handles retain the whole
reference supply; required deferred payloads/defaults follow runtime ownership.

Run takes an opaque ActionHandle or nil/null for the default conversation event.
RunLegacy accepts only objects without steps; plan JSON is InvalidJson before
effects. Legacy custom names still pass the runtime's shadowing checks. Ordered
event JSON strings come directly from HostEvent::to_json, with failure/cut flags.
The host performs the returned effects. The session ignores the entire click while a response bundle is blocked, including default and legacy actions.

`RuntimeLimits.evaluation.peakMemory` optionally bounds portable evaluation
reservations (ADR 0011). It defaults to nil/null; existing calls and result
records keep their shape. Memory cuts reuse `sizeLimited`; refused writes keep
state unchanged. This is an evaluation-storage allowance, with the exclusions
in the ADR, rather than a process-heap limit. The generated wasm getter/setter
uses the same `peakMemory` name and defaults to undefined.

Mutations return a single-use ContinuationHandle. Resume true uses current inputs
and the original work and optional memory allowances; false ends it. Pending runs survive draws and
program changes. **Dropping the last native handle releases the pending run.**
Its destructor upgrades a Weak transport and sends a fire-and-forget release;
it never waits, and only the worker removes/drops thread-affine state. It is safe
from any thread and after the session has gone away. pendingContinuations reports
the current slot count; after a handle drop on the same caller thread it also
provides a dispatch barrier for cleanup. Kotlin GC may delay destruction.

maxContinuations defaults to 64 and must be at least 1. At capacity, a **new
embedder-managed suspension evicts the oldest pending outcome**, releasing its program/run on the
worker; the old token reports EvictedHandle. Runs that finish without suspension,
run(nil/null), and runLegacy remain available and do not evict anything.
Eviction does not undo earlier writes or cancel external host I/O. Consumed tokens
report ConsumedHandle; eviction status lives in the token, not an unbounded
worker tombstone table. Dispose releases every pending run and root. Native
capabilities have no safe numeric constructor; generated unsafe/testing handle
constructors are outside the application API.

A binding marker is a bounded expression-write capability for any state target,
using state, statements and query data. writeBinding itself runs no host action.
**Marker JSON can contain an expression that CALLS a declared custom action.**
That expression can store a real custom step in state; a later draw makes it
clickable with arguments chosen by whoever wrote the marker. This is upstream's
behavior (ADR 0004). **Apps must accept marker JSON only from the tree they were
given, and never directly from model output or tool data.** Do not offer a generic
untrusted-marker endpoint. A finished custom step merely restored from JSON has
no authority; the runtime alone mints/checks identity for evaluated action calls.

Concurrent calls execute serially on one worker. Dispose waits for accepted calls
and joins the worker. Same-session reentry refuses as described in
[Host library functions](#host-library-functions). After normal disposal, methods
throw Disposed. After a worker panic, the original Worker failure stays sticky for later calls, including
reference updates; it is never disguised as normal disposal. Containment does
not roll back earlier effects. Store admission errors map to RefusedWrite;
expression/action-evaluation failures have the separate Evaluation variant.
Invalid marker shape is InvalidMarker, malformed/wrong-shaped/deep JSON is
InvalidJson, and byte/cut refusals are LimitExceeded.

RuntimeLimits has evaluation, referencePreparation, maxJsonBytes (8 MiB), and
maxContinuations (64), plus optional maxActionPathBytes and maxCacheBytes.
maxCacheBytes caps the tool result data, as JSON, a provider's query cache
holds at once: live cache entries, a previous key's included, and mutation
results. Absent, it is the evaluation result-size ceiling. The effective path
bound is min(maxJsonBytes, maxActionPathBytes); absence preserves the previous
maxJsonBytes bound. Choose e.g. 4096 for applications with short paths. Limits
are fixed for the lifetime. maxJsonBytes bounds
host strings/JSON, accepted program JSON, action path text and explicit binding
annotations. Draws use only runtime limits. JSON reading uses the
core's iterative parser and stack-safe freeing and its evaluation.depth nesting
bound. Reference preparation has independent work/depth bounds. Generations
never wrap.

The Store caps both individual values and **total retained state** at
`RuntimeLimits.evaluation`'s result-size **ceiling**: `resultCeiling`, or the fixed
`resultSize` setting that sets that ceiling, 8 MiB by default. There is no separate
state-cap field. Scaled drawing room (`resultFloor`/`amplification`) and reference
preparation do not change this cap, so drawing a shorter program cannot invalidate
previously admitted state. The runtime measures escaped keys, punctuation and
values; aliases count at every position, and undefined/pristine-only entries
count as null. Empty state costs zero (its exported snapshot still spells `{}`).

Every write route delegates to that Store: set, form/default writes, markers,
action sets/resets and mutation resumes. Refused writes return RefusedWrite (in
ActionOutput.failure for actions), with a reason naming the byte cap. The refused
write leaves snapshot, version and pristine flags unchanged; reset preflights its
whole step. Earlier completed action steps remain applied. Initialize loads the
persisted entries/defaults that fit and returns **each** refused name/error in
InitializationOutput.refused, rather than refusing the entire supply. Accepted
entries can therefore change state even when the same batch has refusals. Store
admission follows [ADR 0005](adr/0005-admit-total-state-incrementally.md); this is
a retained-state cap, not a proof of bounded peak heap for all intermediate work.

Run cargo test --offline -p openui-ffi in debug and release. session_overhead
measures dispatch without timing assertions. Binding generation in its own target
dir works with CARGO_NET_OFFLINE=true and --metadata-no-deps. swift-binding.sh
now checks the generated Swift application signatures; the checker optionally
accepts Kotlin too. On Xcode rebuild with scripts/swift-binding.sh and run swift
test in swift/OpenUINative. On Android rebuild with scripts/android-demo.sh and
run connectedDebugAndroidTest on API 34+. See [ADR 0006](adr/0006-native-runtime-sessions-own-a-worker-and-opaque-capabilities.md) for lifecycle decisions.


## WebAssembly

The same session policy now lives in `openui-session`, shared by both bindings.
See the [WebAssembly application interface](wasm-runtime-binding-api.md) for
camelCase methods, bigint limits, one-thread ownership, output/handle free,
reentry, typed errors and UTF-16 differences. Native interface signatures and
worker behavior above are unchanged. The source checker runs without arguments
and checks both bindings; generated Swift/Kotlin checks remain supported.

JSON export retains the runtime's fallible signatures but uses its default
compatibility serializers. No second size, work or depth quota applies to data
already evaluated or admitted by the Store. Root room excludes the JSON envelope
and cut trees can retain raw props, so exported tree bytes may exceed root room.
Raised evaluation limits remain effective with the default host-input ceiling.
Snapshot JSON continues to omit undefined entries (the runtime Store helper
projects them to null, so the session exports its own shallow view). Empty state
costs zero for admission and always exports `{}`, even with zero result room.

Events serialize before a continuation is published or evicted. Potential
continuation-generation exhaustion is checked before effects or consuming a
pending run; abandoning a pending outcome is still allowed. Runtime LimitError
maps to LimitExceeded in openui-session's single runtime error translator.

Binding configuration retains the existing host JSON/input-size admission and
raw-byte annotation bookkeeping. It does not add evaluation-depth, escaped-name,
or estimated extraction-work quotas. A refused mark changes neither its binding
map nor its annotation bookkeeping. Function registration admits declared names
only, at most 256 names and maxJsonBytes total (UTF-8 name bytes plus 64 per
entry). Replacing a name spends no additional room; refusal preserves it.
Explicit-limit Rust runtime configuration and export APIs remain available to
callers who request an additional quota.


## Host library functions

Native registerFunction(name, callback) takes a LibraryFunction interface whose
call(argumentsJson) returns optional JSON text or throws FunctionError.Thrown.
Wasm registerFunction(name, callback) takes a JavaScript function from JSON text
to JSON text or undefined. Arguments are named, validated by the runtime, and
serialized with its serializer. Results use the session's JSON byte, depth and
work limits before conversion to Value. Undefined differs from JSON null.
No JSON result restores custom-step identity, even when a callback returns a
real custom plan received as an argument. Direct Rust identity implementations
can keep that allocation; the JSON boundary deliberately loses it (ADR 0004).
Normal throws use the runtime's exact `@Name threw: message` diagnostic, including its prop context. These are
runtime-error entries in EvaluationOutput.json, not thrown binding errors.
Neither binding replays calls or pre-evaluates query defaults; the runtime owns
validation, argument caching and lazy defaults across an action run.

A failing host function follows the Rust runtime's ordinary function behavior:
a throw, non-JSON result, panic or over-limit result produces `@Name threw: …`
and null; the run continues. This is a function diagnostic, not a refused session
operation or whole-action rollback. For
`Action([@Set($a,1), @Set($b,@Echo(1)), @Set($c,2)])`, a failing Echo leaves
`{"$a":1,"$b":null,"$c":2}`. Evaluation diagnostics include the prop context;
action execution keeps the same writes, events and flags as a direct Rust
throwing implementation. Both bindings pin the action's continuing writes;
native tests additionally cover panic containment.

Native callbacks execute synchronously on one ordinary-stack callback thread
per session, created lazily after a non-mutating query admits its first registration
and before admission to the live session. There is no
timeout and no thread spawned per invocation. The worker sends JSON and waits;
the callback thread owns accepted implementations and releases replaced and
disposed registrations. The registering caller retains an implementation until
the callback thread takes ownership. A caller-side guard also reclaims it if
Rust submission unwinds before handoff; an already admitted missing handoff
revokes the session with Worker("host function released before registration").
The callback loop replies with that error and keeps serving other jobs.
Name/limit refusals, failed spawn/send
handoffs and queued registrations discarded by a failed worker release on that
caller, with the same reentry fence and panic containment. Caller-side release
completes before registration returns; refused names/limits create no callback
thread. If acknowledgement is lost after ownership transfers, the callback
thread releases that implementation during teardown; disposal/final drop waits
for teardown to finish. The core may grow the worker's stack, so
entering foreign runtimes directly there would be unsafe (Android ART aborts).
Disposal and final drop join the callback thread after worker teardown; sessions
without functions need no callback thread.

Callback transport loss or unexpected callback-thread exit revokes the session:
an operation whose callback handoff fails and later calls return the same typed
Worker error, including
calls that invoke no function and calls after disposal. First-registration
admission queries the live shared core through the same code path as real
registration, so refused names/limits create no thread without duplicating rules.
A failed thread start returns Worker without live admission, preserving state, evaluated handles
and registration room; a later registration can succeed. Normal disposal
still yields Disposed. Host throws and caught host/destructor panics do not kill
the callback thread or revoke the session. Infrastructure fault-injection tests
cover these terminal paths; ordinary host code cannot terminate that thread.
Disposal and final drop join both threads even after failure. This promises no
rollback of earlier runtime or external effects.

A correct implementation gives the same result however long it takes. Any
same-session call on the worker or callback thread, including from a replacement
or disposal destructor, refuses with Worker("session already in use"). Commands
carry active session ancestry across nested calls: A → B → A refuses at the last
call; acyclic calls to another session remain allowed. Foreign destruction of
jobs refused on native caller threads uses the same scoped thread-local guard.
Native dispose is infallible and cleanly does nothing on such reentry. Refused
reentrant operations change no session state. Host panics, including destructor
panics, are caught on the thread that enters host code.

Calls from other threads queue in submission order while a callback runs.
Disposal from another thread closes submission, queues shutdown after accepted
calls, and waits for the callback and worker destruction to finish. Concurrent
disposers wait for the same join. Transport locks are released before waits,
joins and foreign destruction. A callback or destructor must not wait for another
thread to call this same session: that call queues behind the callback/destructor,
so the host would deadlock itself. Ancestry follows synchronous binding calls;
it does not follow arbitrary host-created threads. A host function that never returns keeps its caller
waiting, and with it every other thread's call to that session and any disposal
or final drop from another thread; the binding does not cancel host code or undo
external effects.

JavaScript callbacks execute synchronously under the package guard. Reentry and
free/dispose refuse with Worker("session already in use") before entering Rust,
leaving the session usable. As with any synchronous JavaScript call, a callback
that blocks the JS thread cannot be preempted; the host must return for progress.
This synchronous contract requires no asynchronous replay or timing-dependent
result. #60's same-realm private-helper hardening remains separate and open.

## Data-backed elements and retention

As upstream does, element-shaped state, query results and tool output can draw
live elements. Their built-in plans can run through actionAt: JSON is a
capability to request bounded state writes. Host events are only returned for
the embedder to decide whether to perform; the session does no external I/O.
Finished custom-step JSON never grants custom authority (ADR 0004). Data can
also invoke declared custom action calls during evaluation, which earns real
identity under that ADR. An object literal whose k is an expression kind is
interpreted as that expression: `{k:"Str",v:"y",z:2}` yields "y". It does not
render its extra nested data as children.

The retained tree is a Rust Value graph, not its JSON text. The isolated
retained_root test measures a 10,000-element Each result and pins retained heap
below eight times that sample's serialized size, then checks program replacement
releases it. This is a sample bound, not a universal JSON-to-heap multiplier.
The session keeps one root; old and new roots coexist while the new evaluation
is built. Root retention uses runtime input/result/depth/work bounds (including
raw props retained by cuts); it has no independent heap-byte limit. Keeping its
shared allocations preserves custom identities. Shrinking by rebuilding from
JSON would destroy those identities and duplicate work. Applications needing
less retained memory can lower evaluation limits or replace the program.

Oldest-first continuation eviction is deliberately retained: refusal after
running earlier steps would change state despite refusing the call. Refusing
all runs before execution when full would also stop runs that never suspend.
The existing eviction test pins that non-suspending runs keep working. No second
eviction policy is exposed.

For evaluation-memory sizing, profile identical inputs/callbacks in Rust with
`Limits::with_peak_memory(usize::MAX)` and read `EvaluationMemory.accounted_peak`
from `evaluate_tree_with_memory`, `EvaluationSession::memory` or
`ActionRun::memory`. Include all resumed segments and use at least the peak of an uncut profile.
The conservative cumulative account is not a heap multiplier. Binding result
records report flags, not accounted bytes; a binding-only host can bisect the
smallest uncut allowance for a fixed input. `referencePreparation.peakMemory`
is rejected by session construction with the existing `LimitExceeded` error,
including zero and the maximum value; no session or Store state is changed.
Calls that omit it retain their existing behavior.

## Atomic program replacement

`replaceProgram(program, persisted, projected, nativeView)` is one synchronous
operation shared by native and wasm. It stages the Store, including private
custom-action and undefined-field marks, while sharing admitted Values and the
session identity. It projects optional edited-state JSON with the candidate's
defaults, then initializes persisted JSON/defaults, evaluates and serializes the
tree and snapshot. Only success publishes the Store, program, root and generation.
It returns `ReplacementOutput` with initialization/projection results, evaluation,
snapshot and version, so the host needs no fallible reads after commit. There is
no pending transaction to abandon; [ADR 0012](adr/0012-atomic-session-program-replacement.md)
records the choice. Existing setProgram/initialize/evaluate retain their behavior.

Refusal preserves state values/order/version/pristine status, program, handles,
bindings, registrations, continuation accounting and callback-thread identity.
No evaluation work or room is refreshed on existing continuations. Successful
replacement advances evaluation generation twice, exactly as setProgram followed
by evaluate. Continuations remain usable with the new program. Initialization's
per-entry refusals do not throw or roll back the accepted batch; diagnostics and
cut flags remain successful evaluations. Release notifications are drained after
success, never by the new operation's refusal path. Callback delivery stays on
the existing callback thread (ADR 0009); host effects cannot be rolled back.

`nativeView: true` opts into OpenUINative's existing decoder admission before
commit: a null/absent or exactly shaped element root and JSON values below depth
512, including tree/snapshot envelopes. Wrong root shape throws Evaluation;
excessive depth throws LimitExceeded. False admits general runtime output and
adds no display restriction. This is not a new default evaluation limit. Swift
show uses true and checks its generation before calling. The same encoded bytes
are decoded after commit; there are no additional session calls or host-selected
limits on that path. The old throwaway persisted pre-check is removed: JSON is
admitted once using the actual session's prompt bindings and immutable limits.

## Driving queries

Attach the capability with `attachToolProvider()`. In Swift and Kotlin use
`evaluateWithResponse(responseState)` or
`replaceProgramWithResponse(…, responseState)` for a response in progress.
The original settled signatures remain callable through the generated Swift
protocol and Kotlin interface. Swift protocol requirements cannot declare
default arguments. JavaScript accepts an optional response state on its
existing `evaluate()` and `replaceProgram(…)` methods.

Pass `streaming` and the bundle reader’s `isBundle`, `complete`, `error`,
`scripts` and `scriptRevision`. In JavaScript the object can be built as
`{ ...JSON.parse(parseResponseBundle(raw, streaming)), streaming }`.
Parse the returned `program`. A bare program needs only `streaming`.
The session defers query evaluation and mutation registration while streaming,
incomplete, or reporting a bundle error. A blocked first draw reads a
provider query as null even if the query declares a default. Without a provider the draw uses the
declared default. These are the renderer's respective behaviors. An edit
keeps live registrations and accepts outstanding outcomes while blocked.
Settling registers the current statements, including an empty list.
`setProgram` waits for a draw before registering queries. Between the program
change and the next successful draw, clicks and Retry obey the last draw's
bundle gate.

The session retains the last settled Cloud script names and opaque revision
during an edit. A query calling one of those names includes
`__openuiScriptRevision` in its cache key. To supply application values on
the attached provider, call `setQueryExtraValues(toolName, valuesJson)` with
a JSON object. Upstream’s renderer supplies the script dependency through
lang-core’s single `extraDeps` callback. The session combines both inputs
at the same point. On the next eligible evaluation the query manager merges
declared dependencies, application values, then the script revision. A later
value wins on a repeated key. An empty object clears the app values
for that tool. Changing only query keys preserves registered mutation tool names.

Read `evaluation.blocked` to draw buttons and inputs disabled. It is the
renderer’s `blocked` for the response the draw was given: true while it
streams, and for a response bundle that is incomplete or has an invalid
scripts section, even once streaming ends. Upstream hands this value to its
components as `isStreaming`. A draw without response state reports false.
The flag only reports: the session ignores clicks and Retry for a blocked
response bundle whether or not a control was disabled, and still takes a
click on a streaming bare program, as upstream does
([ADR 0016](adr/0016-every-draw-reports-whether-its-response-is-blocked.md)).

Read `evaluation.isQueryLoading` for the renderer’s loading indicator. It
also becomes true before dispatch for a streaming response bundle that
declares queries. An invalid scripts section still draws its program.
`evaluation.parserErrorJson` holds the separate parser diagnostic with code
`parse-failed` and the message from the reader. Response metadata must fit
`maxJsonBytes`, counted in UTF-8 bytes. The WebAssembly package also holds
the scripts JSON it hands to Rust to `maxJsonBytes` bytes. A parser diagnostic that exceeds the evaluation limits
refuses the draw. A revision that exceeds query-key limits reports
`queryEvaluationError` and preserves the previous registrations.

The tree JSON keeps its `{root,errors}` shape. The separate query snapshot
includes results, `__openui_loading`, `__openui_refetching` and
`__openui_errors`. A successful draw publishes a new generation. A refused
draw preserves the previous root and its evaluation and action handles.
Even a provider-free session refuses a draw with `LimitExceeded` if its
64-bit evaluation generation is exhausted. Ordinary tree work and depth
cuts return a successful draw with the corresponding flags.
Query registration hitting a limit is a successful draw with the previous
registrations and a typed `queryEvaluationError`. State edits can recover
the registration and clear the error. Draining effects does not clear the
diagnostic on screen.

Perform each wanted call and return its opaque object to `completeToolCall`
with an outcome JSON object containing exactly one of these fields.
`completeToolCall` refuses an outcome over the limits and keeps the receipt,
so the embedder can send a corrected one. `finishToolCall` takes the same
outcome as final: a refused outcome, including invalid JSON or a failure
message too large to admit, settles the call as a tool error naming the
reason. The JavaScript driver's terminal mode (`completeToolCall`'s third
argument) is the same operation.

| Field | Meaning |
| --- | --- |
| `value` | Result value, including explicit null |
| `undefined` | A missing result (pass `true`). Queries cache null and successful mutations omit `data`. |
| `availableTools` | Missing tool, with available names in provider order |
| `error` | Tool-reported MCP error text (`mcp-error`; the MCP helper's `ToolReported` outcome) |
| `failed` | Call failure message |
| `mcpResponseJson` | Raw MCP response JSON text. Extracts the result inside Rust and preserves JavaScript numbers and lone surrogates through delivery. |

After any operation, call `takeQueryUpdate()` to receive the wanted calls
and redraw flag. Writes, action runs, and completions can all use the same
driver loop. A query completion dispatches its remembered run first, then
any calls from readers whose inputs changed. A run of a waiting reader does
nothing. A reader with a fetched input can run while its source refetches.
A fetched null releases a reader. A default does not. During a blocked edit, readers
from the last settled registration still read the live Store and use its Cloud
script revisions. Completing an outstanding source call can dispatch such a
reader before the edited bundle settles.
Settled state changes evaluate queries before returning.
`changed` accumulates changes to the query snapshot, its presence on attach/detach, or its evaluation
error since the previous drain. The drain clears this flag and transfers
the calls once. `readerRoundsLimited` reports reader inputs left changing
at the statement-count-plus-one round bound. A result refusal still ends
its call if the previously bounded readers cannot run with the available
context. In that case the update carries their cut flag forward.
The drain clears this flag. Its `evaluationError` gives the current diagnostic.
If a provider success fits but evaluating dependent arguments exceeds the work
or size allowance, completion consumes the receipt and keeps the admitted result.
The source is visible when its own inputs are still current.
Affected readers and their dependents display null and a `reader-limit` query
error. If listing all the errors would itself exceed the allowance, the snapshot
omits the suspended results and errors. The draw and drain report `LimitExceeded`.
A successful settled registration retries those expressions and clears the diagnostic.
A result that exceeds admission ends its call as a tool error. A dependent
query waits for its source's first admitted result. If the source already
has data, a refused refetch leaves the dependent on its current arguments.

A query can be edited to read its own previously fetched result. When its
new result differs from the self-input used to call the tool, the draw and
drain report an evaluation diagnostic naming the query. The displayed value
was fetched with the previous self-input. An explicit settled registration
evaluates the arguments again.

With a provider, a settled program registers mutation tool names. For both
queries and mutations the wanted call exposes the JavaScript string form of
the evaluated tool name. For example, `Mutation(7)` dispatches `"7"` and a
structured error keeps the number `7` in `toolName`. While a bare program
streams, a click uses the last registered mutation tool and evaluates the
arguments in the current program. An unregistered mutation ends the plan at
that step. A dispatched mutation suspends the plan.
A second click while loading ends that second plan at the Run step. Events
preceding the Run are in the click's ordinary action output.

Report the mutation's outcome with `completeToolCall`, then take the same
query update. Its `actions` list holds each resumed action output in completion
order. WebAssembly exposes `actionCount` and `actionAt(index)`. A successful
outcome runs the tail against the current program, state and results. A failed
outcome ends the plan and yields an empty segment. Read the snapshot for the
mutation's error. Query arguments can read an idle or loading mutation.
Mutation completion alone does not re-evaluate those arguments. A resumed
state write can trigger registration with the new mutation result.
A later action step can itself fail, and its segment includes
the earlier events and the failed step. A query Run before the mutation
takes effect immediately. One after the mutation runs only on success. In
either segment, query runs use their registered arguments before a state
write triggers query evaluation. A tail can also suspend again or change
state that requests more calls. The same drain transfers all of these effects
once. Its redraw flag includes a resumed store write and a mutation reference
change hidden by a reserved snapshot list.

Each managed submission occupies one `maxContinuations` slot (default 64)
until the session accepts its outcome, queries pause, or the provider detaches. Draining or
dropping the wanted call keeps its slot occupied. At capacity, a Run that
would dispatch a new call reports `LimitExceeded` and a size cut. Existing
plans remain pending.

A driver that cancels a call while the provider stays attached can report
`{"failed":"cancelled by provider"}` to `completeToolCall` with the original
call handle. This ends its plan and frees one slot. If its registration
is still live, the mutation records a `tool-error`. A later outcome for
that handle is refused.
If the driver drops a handle without reporting an outcome, detaching the
provider releases all pending managed plans. Detaching preserves already
completed segments for the next drain. Without a provider, a mutation Run emits its host event and gives the
embedder its ordinary continuation.

With a provider, a `@Run` step naming a query runs that query again and
emits no host event; the other steps' events are returned as before. A run
while that query's key has a call in flight is remembered once for the key,
whichever query started the call, and its call appears in the drain that
follows the outcome. `retryFailedQueries()` runs again every query whose
error appears individually or in a compact diagnostic. With
no query errors it runs every query. Both act on the
queries registered so far even while the response state is streaming, as
upstream's do for a bare program. The session ignores both while a response
bundle is blocked. Without a provider, a `@Run` of a
query is still the host's `run` event.

Skipping a drain leaves still-pending calls parked in dispatch order.
Later operations discard calls whose entries have been released. The
manager retains at most a current and previous cache entry per registered
query, each with at most one pending call. Detaching discards the cache
and revokes calls. A repeated or foreign completion raises `InvalidToolCall`.
A result too large or too deep for `RuntimeLimits.evaluation`, or past the
cache cap, is the tool's: it is delivered as the call's `tool-error`, naming
the limit, on every query of its key (or as the mutation's error, which ends
its plan), and the call is consumed. Nothing held is evicted. Malformed or
wrongly shaped outcome JSON (at any length, MCP responses included) and
failure text past the limits are the embedder's: the outcome is refused and
leaves a valid call available for correction. A delivery the query
snapshot's room cannot hold still ends the call, as each reader's
`tool-error` or one compact error. Outcome text may
be as long as the smallest of maxJsonBytes, the result-size ceiling and the
work limit, plus the 10 bytes of `{"value":}`; a longer result is refused for
size without being parsed. A result may nest to `evaluation.depth` levels.
See [CONFORMANCE.md](../CONFORMANCE.md#tool-result-limits). Supplied
references conflict with an attached provider. Clear them with
`setReferenceValues("{}")` before attaching.


WebAssembly applications can delegate this drain and its timer to the package
driver. The driver uses terminal completion so an oversized provider
diagnostic also settles the call. [JavaScript tool provider](wasm-runtime-binding-api.md#javascript-tool-provider)

### Scheduling one refresh timer

Attach or reattach with `attachToolProviderAt(nowMs, minimumMs)` to set the
monotonic integer millisecond clock before the first registration. Use
`resumeQueriesAt(nowMs)` on returning from the background. The unit is `UInt64`
in Swift, `ULong` in Kotlin and `bigint` in JavaScript. Read `nextRefreshDueMs` from every
`takeQueryUpdate()`, even when `changed` is false. It gives an absolute
deadline on that same clock. Arm one platform timer for the remaining delay,
or cancel it when the deadline is absent. On wake, call `advanceQueryClock(nowMs)` with the actual
time and drain again. A repeated drain keeps reporting the current deadline.

The default polling floor is 1,000 ms. Choose a floor from 1 through
2,147,483,647 ms. A repeat attachment with an explicit minimum must match
the attached manager or it raises `RefusedWrite` before changing the clock
or calls. Detach to choose a new minimum. Plain `attachToolProvider()`
keeps any existing floor. When attaching a fresh provider without a timestamp,
`attachToolProvider()` and `attachToolProviderWithMinimum(minimumMs)` start
at zero. Advance before the first draw if using a host clock with those methods.
The floor applies only to interval ticks.
Advancing to a time earlier than the previous advance, or above
`UInt64.max - 2,147,483,647`, raises `LimitExceeded`. The upper bound leaves
room for one more full platform interval. A delayed wake runs each due timer
once, in deadline order with ties in installation order. Each timer waits a full period from that wake,
including when a call in flight made it skip fetching.

Use `pauseQueries()` to stop the timers and release outstanding queries and
managed mutation plans. Each original call identity can still acknowledge one
late outcome, which changes nothing. The timestamped attach and resume methods also advance an already
active manager. The zero-argument `resumeQueries()` uses the last supplied time.
Resume re-evaluates a settled program using its retained cache. A streaming program waits for its settled draw.
During ordinary streaming, previously registered timers continue with their
last evaluated arguments for both bare and bundled responses.

An action that loses its mutation to a pause is reported once, as an action
failure whose error is `QueriesPaused` (#117), and nothing more of its plan
runs, after resume either. A mutation step reached while paused (a click, or
a host continuation resumed then) fails in that call's own `ActionOutput`: no
call is made, the steps before it keep their effects and `failure.step` names
it. A plan whose mutation call was in flight when queries paused comes back
from the next `takeQueryUpdate` as one action segment with that failure, no
events and no continuation, in click order. The failure is not a limit, so
`sizeLimited` stays false. A query `@Run` while paused starts no call and the
plan carries on, as upstream's does.

The reason is one of two fixed texts, and says what the session knows:

- "queries are paused, so the mutation was not sent to the tool provider and
  the rest of the action did not run": the call was never handed to a driver.
  Either the step ran while paused, or its call was still among the calls the
  session had not yet handed over by `takeQueryUpdate` when the pause cleared
  them. No provider can have been asked to run it.
- "queries were paused after the mutation's call was handed to the tool
  provider, so the tool may have run; its outcome is ignored and the rest of
  the action did not run": a driver had taken the call. Whether the driver had
  invoked the provider yet, the session cannot know. The JavaScript driver
  invokes the provider in the drain that takes the call; OpenUINative starts a
  task that does not invoke it once cancelled by the pause. Either way the
  late outcome is ignored and nothing of the plan runs.

A failure waiting for the next drain holds a continuation slot. A managed
submission is refused past `maxContinuations` (it fails at its mutation step
with the pending-plan limit), so at most that many failures wait. A host
handoff counts them too and evicts the oldest pending run when the slots are
full, but is never refused; with only failures in the slots it adds one, so
pending runs plus waiting failures are at most `maxContinuations + 1`. While
the program is also unregistered (a refused registration, ADR 0013) the
mutation step fails for that limit instead, once.


### The Swift driver

Give `LiveProgram.show(response:streaming:)` the entire raw response so far. A potential marker line is held back while streaming. The last content section wins, and the scripts section is removed before parsing. The Swift preview keeps its existing interaction gate, including for a bare program. `live.isQueryLoading` includes a streaming response bundle that declares queries. Once streaming ends, an invalid scripts section still draws its program and exposes the session diagnostic in `live.parserError`. The next accepted step replaces the displayed response metadata. The session's `StreamSession` reads it with `readResponse(text:streaming:)`, which remembers how far it read: a response that only grows costs about what pushing it does, and any other edit is read afresh, with the same result ([ADR 0015](adr/0015-an-incremental-reader-for-a-raw-response-that-grows.md)). It returns `parseResponseBundle`'s JSON, `parseMessage`'s context and attributes (when the text has a marker) and the program's parse in one call; the parse is the session's own, so `finish`, `push` and `set` go on from it.

Pass one async closure to `LiveProgram.attachToolProvider`. It receives the tool
name and optional `JSONValue` arguments and returns a `JSONValue`. Wanted calls
run concurrently. Each completion updates the evaluated tree and the loading,
refetching and error lists in `live.queries`. Its `values` object maps statement
IDs to their current values, including query defaults and mutation status.
Resumed host events go to the same `onHostEvent` handler as a click.

| Provider outcome | Session outcome |
| --- | --- |
| Returned value | `value` |
| `ToolProviderError.toolNotFound(availableTools:)` | `availableTools` in the given order |
| `ToolProviderError.toolReported(_:)` | `error` |
| `ToolProviderError.failed(_:)` or another thrown error | `failed` with the message |
| Provider cancellation | `failed` with `cancelled by provider` |

A refused oversized value is reported with the session's limit error.
`retryFailedQueries()` delegates Retry to the session. The app's streaming
request and stream previews defer new registrations. Outstanding calls and
registered refresh intervals can continue while streaming. The live program
registers settled arguments when it finishes or discards a preview and the
app's streaming request is clear.

The default clock is `ContinuousClock`. Pass a clock and an optional
`minimumRefreshInterval` to `attachToolProvider(clock:minimumRefreshInterval:_:)`
to drive time explicitly. The driver arms one timer at the session's deadline
and reports the actual time when it wakes. The session chooses the next
deadline. The driver applies no timeout.

The last `ElementView(live:)` disappearing pauses queries. The first one
returning resumes them unless the app called `pauseQueries()`. The app releases
its pause with `resumeQueries()`. Pausing, detaching and releasing the live
program cancel outstanding work. A provider that returns after cancellation
cannot deliver a result or a resumed host event to that program. A mutation
dropped by the pause, clicked while paused or in flight when it came, is
reported as an action failure in `live.failure`, as `Step N: ` and the
`QueriesPaused` reason, the way other action failures are; its host events
are never delivered.
