# Swift and Kotlin runtime sessions

The [generated application signatures](runtime-binding-api.md) list constructors,
methods, records and typed errors. Create RuntimeSession for a ComponentLibrary;
its original schema supplies action definitions. Supply the prompt spec's library
object or full wrapper for BindingProps. undecidedBindings reports ambiguous
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
The host performs effects. **Ignoring clicks while a response is bundleBlocked
is the embedder's job.**

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
suspension evicts the oldest pending outcome**, releasing its program/run on the
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
maxContinuations (64), plus optional maxActionPathBytes. The effective path
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

The response state currently carries one boolean, `streaming`. For a
bundle, pass the upstream renderer's combined gate in that field,
`isStreaming || !bundle.complete || !!bundle.error`. The caller still
owns the bundle. A blocked first draw reads a provider query as null even
if the query declares a default. Without a provider the draw uses the
declared default. These are the renderer's respective behaviors. An edit
keeps live registrations and accepts outstanding outcomes while blocked.
Settling registers the current statements, including an empty list.
`setProgram` waits for a draw before registering queries.

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

| Field | Meaning |
| --- | --- |
| `value` | Result value, including explicit null |
| `availableTools` | Missing tool, with available names in provider order |
| `error` | Tool-reported MCP error text |
| `failed` | Call failure message |
| `mcpResponseJson` | Raw MCP response JSON text. Extracts the result inside Rust and preserves JavaScript numbers and lone surrogates through delivery. |

After any operation, call `takeQueryUpdate()` to receive the wanted calls
and redraw flag. Writes, action runs, and completions can all use the same
driver loop. Settled state changes evaluate queries before returning.
`changed` accumulates changes to the query snapshot, its presence on attach/detach, or its evaluation
error since the previous drain. The drain clears this flag and transfers
the calls once. Its `evaluationError` gives the current diagnostic.

With a provider, a `@Run` step naming a query runs that query again and
emits no host event; the other steps' events are returned as before. A run
while that query's key has a call in flight is remembered once for the key,
whichever query started the call, and its call appears in the drain that
follows the outcome. `retryFailedQueries()` runs again every query whose
error is in the query snapshot, as the upstream renderer's Retry does; with
no query errors it runs every query, as upstream's does. Both act on the
queries registered so far even while the response state is streaming, as
upstream's do for a bare program. Upstream ignores clicks and Retry while a
bundle is blocked; that stays the embedder's to do until the session knows
the bundle's state. Without a provider, a `@Run` of a
query is still the host's `run` event.

Skipping a drain leaves still-pending calls parked in dispatch order.
Later operations discard calls whose entries have been released. The
manager retains at most a current and previous cache entry per registered
query, each with at most one pending call. Detaching discards the cache
and revokes calls. A repeated or foreign completion raises `InvalidToolCall`.
A refused outcome leaves a valid call available for correction. Supplied
references conflict with an attached provider. Clear them with
`setReferenceValues("{}")` before attaching.
