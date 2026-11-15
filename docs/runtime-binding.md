# Swift and Kotlin runtime sessions

The [generated application signatures](runtime-binding-api.md) list constructors,
methods, records and typed errors. Create RuntimeSession for a ComponentLibrary;
its original schema supplies action definitions. Supply the prompt spec's library
object or full wrapper for BindingProps. undecidedBindings reports ambiguous
pairs; markBinding resolves them. Declared library functions have no host
implementations here and return null. Synchronous callbacks remain a follow-up.

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
buffered.push(chunk) // ParseOutput?, null when holding the entire chunk
buffered.finish()    // throws MalformedInputException for an unmatched final unit
```

Its Android test checks direct push rejection, a split emoji pair, an empty
intervening chunk, end-of-stream rejection and unchanged state after failed
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

Mutations return a single-use ContinuationHandle. Resume true uses current inputs
and the original work allowance; false ends it. Pending runs survive draws and
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

Concurrent calls serialize on one worker. Dispose waits for an in-flight call
and joins it; final drop does the same from any thread. There are no synchronous
host callbacks. After normal disposal, methods throw Disposed. After a worker
panic, the original Worker failure stays sticky for later calls, including
reference updates; it is never disguised as normal disposal. Containment does
not roll back earlier effects. Store admission errors map to RefusedWrite;
expression/action-evaluation failures have the separate Evaluation variant.
Invalid marker shape is InvalidMarker, malformed/wrong-shaped/deep JSON is
InvalidJson, and byte/cut refusals are LimitExceeded.

RuntimeLimits has evaluation, referencePreparation, maxJsonBytes (8 MiB), and
maxContinuations (64). Limits are fixed for the lifetime. maxJsonBytes bounds
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
map nor its annotation bookkeeping. Library function callbacks remain
unregistered, so these bindings have no function-registration call to adapt.
Explicit-limit Rust runtime configuration and export APIs remain available to
callers who request an additional quota.
