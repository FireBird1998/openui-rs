# WebAssembly runtime application interface

Build with `scripts/wasm.sh`; import from `target/wasm-pkg/web/openui_wasm.js`
(browser/bundler, await its default initializer) or require
`target/wasm-pkg/node/openui_wasm.js` (Bun/Node, already initialized).
The build appends the required lifetime/reentry guard, checks the generated
TypeScript declarations and loads both generated packages (the guard refuses to load
if a generated public member is not covered). Generated private `__*` methods
and pointer fields are outside this interface: the helpers throw InvalidHandle
outside a guarded call and pointer fields refuse a new value, which stops
mistakes. It is not a boundary against hostile JavaScript in the same realm
(ADR 0007): isolate untrusted scripts from this package.

JavaScript sees these methods (positional arguments). `?` here means omitted,
null or undefined. JSON arguments/results are strings. Integers in limits,
versions and pending counts are bigint. failure.step is an ordinary JSON number
(null when absent).

```typescript
new ComponentLibrary(schema: string)
parse(program: string, library: ComponentLibrary, rootName?: string): string
parseProgram(program: string, library: ComponentLibrary, rootName?: string): ParsedProgram
new StreamSession(library: ComponentLibrary, rootName?: string)
// Existing push/feed/set/result/finish still work; new retained handoff:
stream.program(complete: boolean): ParsedProgram

new RuntimeSession(library: ComponentLibrary, promptSpec?: string)
RuntimeSession.withLimits(library: ComponentLibrary, promptSpec: string | undefined, limits: RuntimeLimits): RuntimeSession
RuntimeSession.extractMcpToolResult(responseJson: string, limits: ParseLimits): string
session.setQueryExtraValues(tool_name: string, values_json: string): void
session.attachToolProvider(): void
session.detachToolProvider(): void
session.completeToolCall(call: WantedToolCall, outcomeJson: string, terminal?: boolean | null): void
session.takeQueryUpdate(): QueryEffects
session.retryFailedQueries(): void
session.advanceQueryClock(now_ms: bigint): void
session.pauseQueries(): void
session.resumeQueries(): void
session.resumeQueriesAt(now_ms: bigint): void
session.attachToolProviderAt(now_ms: bigint, minimum_ms: bigint): void
session.attachToolProviderWithMinimum(minimum_ms: bigint): void
session.actionAt(evaluation: EvaluationHandle, path: string[]): ActionHandle
session.dispose(): void
session.evaluate(responseState?: ResponseState): EvaluationOutput
session.get(name: string): string | undefined
session.getField(form: string | undefined, name: string): string | undefined
session.initialize(persisted?: string): InitializationOutput
session.initializeField(form: string | undefined, componentType: string | undefined, name: string, json: string | undefined, streaming: boolean): FieldWriteOutput | null
session.isPristine(name: string): boolean
session.markBinding(component: string, prop: string): void
session.registerFunction(name: string, callback: (argumentsJson: string) => string | undefined): void
session.replaceProgram(program: ParsedProgram, persisted: string | undefined, projected: string | undefined, nativeView: boolean, responseState?: ResponseState): ReplacementOutput
session.pendingContinuations(): bigint
session.resume(continuation: ContinuationHandle, succeeded: boolean): ActionOutput
session.run(action: ActionHandle | undefined, message: string, form?: string): ActionOutput
session.runLegacy(json: string, message: string, form?: string): ActionOutput
session.set(name: string, json?: string): boolean
session.setField(form: string | undefined, componentType: string | undefined, name: string, json: string | undefined, save: boolean): FieldWriteOutput
session.setProgram(program: ParsedProgram): void
session.setReferenceValues(json: string): void
session.snapshot(): string
session.undecidedBindings(): {component: string, prop: string}[]
session.version(): bigint
session.writeBinding(marker: string, json: string): boolean
session.free(): void
```

The static MCP helper returns either `{"value": …}` or `{"error": "…"}` as
JSON text. It can be called before creating a session. Malformed JSON,
JavaScript TypeErrors, and limit refusals throw `InvalidJson`, `Evaluation`,
and `LimitExceeded`, respectively. The helper uses the result-size ceiling,
work and depth from the supplied `ParseLimits`.

`new ParseLimits()` has optional bigint properties programSize, resultSize,
resultFloor, amplification, resultCeiling, depth, work, peakMemory; defaults are undefined.
`new RuntimeLimits()` has evaluation and referencePreparation (ParseLimits),
maxJsonBytes (8388608n), maxContinuations (64n), optional maxCacheBytes (the
query cache's byte cap, absent for the result-size ceiling), optional maxActionPathBytes
(undefined). Set e.g. 4096n for a smaller opt-in retained-path cap. referencePreparation.depth is
32768n; all other ParseLimits fields default to undefined. Limit getters return
copies: edit one, assign it back, then free the copy. `ParseLimits.peakMemory`
is an optional bigint (undefined by default), measured in portable reservation
bytes under ADR 0011. Set it on `RuntimeLimits.evaluation`; resumes keep one
run-wide allowance. It is not a process-heap limit. Limits are fixed when a
session is constructed. `ActionHandle.retain()` returns another opaque token;
the package uses this to borrow an optional action without consuming the caller's
object (wasm-bindgen cannot lower Option<&ActionHandle>).

```typescript
interface EvaluationOutput {
  readonly json: string; readonly sizeLimited: boolean; readonly depthLimited: boolean;
  readonly isQueryLoading: boolean; readonly parserErrorJson: string | undefined;
  readonly blocked: boolean;
  readonly querySnapshotJson: string | undefined;
  readonly queryEvaluationError: {kind: string; message: string} | null;
  readonly evaluation: EvaluationHandle;
  free(): void;
}
interface WantedToolCall {
  readonly toolName: string; readonly argumentsJson: string | undefined;
  free(): void;
}
interface QueryEffects {
  readonly readerRoundsLimited: boolean;
  readonly nextRefreshDueMs: bigint | undefined;
  readonly changed: boolean; readonly callCount: bigint; readonly actionCount: bigint;
  actionAt(index: bigint): ActionOutput | undefined;
  readonly evaluationError: {kind: string; message: string} | null;
  callAt(index: bigint): WantedToolCall | undefined;
  free(): void;
}
interface ResponseState { streaming: boolean; isBundle?: boolean; complete?: boolean; error?: string | null; scripts?: string[]; scriptRevision?: string | null }
interface ReplacementOutput {
  readonly initialization: InitializationOutput; readonly projection: InitializationOutput;
  readonly evaluation: EvaluationOutput; readonly snapshot: string; readonly version: bigint;
  free(): void;
}
interface ActionOutput {
  readonly events: string[]; readonly sizeLimited: boolean; readonly depthLimited: boolean;
  readonly failure: {step: number | null; error: {kind: string; message: string}} | null;
  readonly continuation: ContinuationHandle | undefined;
  free(): void;
}
interface InitializationOutput {
  changed: boolean;
  refused: {name: string; error: {kind: string; message: string}}[];
}
interface FieldWriteOutput { changed: boolean; saveRequested: boolean }
```

Output objects own native resources too. Read the handle once, then free the
output when finished with its fields. Each handle getter returns another token;
free every token you retain. Freeing the last continuation token and its output
releases the pending run on that thread during destruction. A weak cleanup guard
runs after the token field drops; it cannot retain a session or panic on a busy
borrow. A busy Rust borrow defers cleanup until the next operation;
pendingContinuations remains a cleanup barrier. GC may delay destruction; dispose/free drops the entire session
immediately. Evaluation/action tokens retain no evaluated tree. Handle objects
and ParsedProgram have free(), with no application constructor. Every object's
`[Symbol.dispose]` is the same guarded free, so `using` declarations work.
Session free and dispose are idempotent. After free (or Symbol.dispose), every
public method and property throws Disposed and freeing again does nothing; a
freed object passed as an argument throws Disposed before wasm lowering. After
dispose, calls into the live wrapper throw Disposed from Rust.

Arguments are checked, not coerced: a string parameter given a non-string,
or a boolean parameter given a non-boolean (including `undefined` for
`succeeded`, `save`, `streaming` and `complete`), throws InvalidJson; a limit
that is not a bigint in 0..2^64-1 throws LimitExceeded; a handle that the
package did not return, of the wrong class, throws InvalidHandle. No
`valueOf`, `toString` or `length` of an argument ever runs. An action path is
copied (its elements may be getters; they run under the guard) and each
segment's lone surrogates become U+FFFD, as for every other string.

All fallible session calls throw Error with a stable string `kind` and `message`.
The guard builds that Error after Rust returns (`kind` is defined as an own
property; no setter runs) and parses records with intrinsics captured at
load, so patched globals cannot change a result or strand a session.
An unexpected failure inside generated glue is reported as Worker.
Kinds match native: Disposed, ForeignHandle, StaleHandle, ConsumedHandle,
EvictedHandle, InvalidHandle, InvalidMarker, InvalidToolCall, ReferenceConflict, InvalidJson, RefusedWrite,
Evaluation, LimitExceeded, Worker, InvalidSchema, InvalidPromptSpec, Prompt,
QueriesPaused. Failures inside actions and initialization appear in the records above, with
that same kind/message shape. `QueriesPaused` is only ever an action's
failure: its mutation was dropped because queries are paused (below). Reentry uses Worker("session already in use")
without effects; it does not revoke an otherwise usable session. The package
guard includes JavaScript argument conversion, so an action-path Array getter
cannot reenter or free an object while a call is lowering, and registered callbacks execute under that same guard. The Rust inner type
also uses try_borrow_mut. Registered library callbacks are synchronous.
Native panic-containment can be tested on the inner type; unexpected wasm
panics cannot unwind under wasm's abort strategy, so this is not a recovery
mechanism for arbitrary implementation bugs.

wasm-bindgen converts JavaScript UTF-16 strings lossily to UTF-8. A lone
surrogate arrives at the session as U+FFFD, including in state names and JSON
text. To preserve a lone code unit, pass ASCII JSON escapes, for example
`session.set('$text', '"\\ud800"')`. This differs from Kotlin's rejecting encoder;
a split surrogate pair in source chunks must be buffered by the caller.

See [session semantics](runtime-binding.md) for authority, marker trust,
streaming/pristine rules, host events, the session’s gate on clicks and Retry, and state
admission. Evaluation JSON and each event JSON are the runtime serializer's
bytes; no JavaScript reserialization changes them. All execution happens on one
thread. Native worker failures remain Worker; wasm has no worker/transport
creation failure. Reference values are prepared once on setReferenceValues using
PreparedReferenceValues::owned_with_limits and reused by evaluate, writeBinding,
run, runLegacy and resume, with the same receipts and cut flags as native.


registerFunction accepts only declared names and a function callback. Arguments
and results are JSON text within session JSON limits; undefined is distinct from
"null". A thrown Error's message, or String(thrownValue), becomes the runtime's
exact `@Name threw: ...` diagnostic. If extracting that text itself throws
(for example a throwing toString, a Symbol-valued Error.message, or a Proxy
trap), the exact fallback is `host function threw`. Finished custom-step JSON returned by a callback never restores identity;
subsequent evaluation of declared calls follows ADR 0004. Invalid result JSON also produces a runtime-error diagnostic and null.
Every public method, including free/dispose, refuses reentry during a callback
with Worker("session already in use") and no effects. The guard is active for
all registered callbacks; try_borrow_mut additionally protects the Rust adapter.
A synchronous JS callback that blocks cannot be interrupted; its host must
return for the session to progress. See [the complete callback contract](runtime-binding.md#host-library-functions).

The build now loads and initializes both Node and web packages and exercises a
guarded session from each. [check-wasm-load.mjs](../scripts/check-wasm-load.mjs)
also pins the Disposed error message; built-package tests pin runtime throw,
reentry and path-refusal messages exactly.

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

Replacement uses the shared [atomic replacement contract](runtime-binding.md#atomic-program-replacement).
Read `replacement.evaluation` once, then free its evaluation handle, evaluation
output and replacement output separately. Initialization/projection getters are
plain JSON records; snapshot remains JSON text. `nativeView` is an explicit
boolean, not an optional coercion or a JSON authority field.

Omit the response state to draw a settled program. A refused draw preserves
the previous root and evaluation handle. On a blocked first draw, a query
with a provider reads as null. Without a provider it reads its declared
default, as in the upstream renderer.

Call `takeQueryUpdate()` to claim wanted calls and the redraw flag. Query
effects and calls are return-only objects with TypeScript type exports.
Use `callAt` with a zero-based bigint. Each object supports `using` or
`free()`. [Driving queries](runtime-binding.md#driving-queries)

### JavaScript tool provider

```typescript
interface RuntimeSession {
    setToolProvider(provider?: ToolProvider | null, options?: ToolProviderOptions): void;
    sampleQueryClock(now_ms: bigint): void;
}
```

Pass a map of functions taking the evaluated arguments, or an MCP client with
a callable `callTool({ name, arguments })`. Return JSON-compatible values
or promises for them. A function-map handler may also return `undefined`.
A later `setToolProvider(provider)`
changes the provider for future calls and keeps the current callbacks.
Supplying options replaces all three callbacks, including for calls already in flight.
Pass `null` to detach it. A manual attachment keeps ownership of its
calls until detach. Installing a driver during that attachment throws
`RefusedWrite`, including when the caller already holds a wanted call.

The driver drains effects after session operations, including
`replaceProgram`, `evaluate`, writes and action runs. For a manually parsed program, pass the application's
response state to each draw. A bare program needs only `{ streaming }`.
A blocked bundle suppresses actions and Retry, including Retry for readers
represented by a compact error. Bare streaming keeps Retry available for
registered queries. Use the draw's `isQueryLoading` and `parserErrorJson`
for bundle loading and parser diagnostics. Read `__openui_loading`, `__openui_refetching`
and `__openui_errors` from the evaluation's `querySnapshotJson`.
Use `retryFailedQueries()`, `pauseQueries()` and `resumeQueries()` on the
same session. While the driver is attached, manual attach methods,
`sampleQueryClock`, `advanceQueryClock`, `resumeQueriesAt`, `takeQueryUpdate`
and `completeToolCall` throw `RefusedWrite`. The driver coalesces completion
drains into one host task. Writes during dispatch, from a provider's promise
reactions, or while reading its result can trigger further calls on the next
event-loop turn. A write made while a call is pending, or while a
completion's turn is queued, therefore dispatches one task later instead of
at once, and so does the next dispatch after pausing or detaching while a
call is pending, or a session operation made from provider code or a driver
callback. That task comes from `setImmediate` where it exists (Bun, Node),
else from a `MessageChannel` (browsers), else from a zero-delay timer; it is
never a microtask, so timers and input run between rounds (ADR 0014). A
source that throws falls back to a timer; if that also throws, `onError`
hears it once and the next session operation drains. On Bun and Node, test
fakes that replace only `setTimeout` do not govern the turn: fake
`setImmediate` as well.

The driver accepts these options.

- `minimumMs` sets the session's refresh minimum in whole milliseconds. Changing the minimum requires detaching first.
- `onChange()` requests a redraw when the drain reports a visible change.
- `onAction(output)` receives each resumed action output. Read its events
  or claim its continuation inside the callback. The driver frees the output
  when the callback returns. An output whose plan stopped carries `failure`;
  that failure also reaches `onError`, once, just before `onAction`.
- `onError(error)` receives query evaluation errors with their binding `kind`
  and message. Repeated drains of the same error report it once, and a
  successful registration clears it. A drain that reaches the reader round limit
  reports a `LimitExceeded` error through the same callback. Timer and binding
  failures and errors thrown or rejected by `onAction` or `onChange` also reach
  it. A callback failure leaves the drain running. The driver contains errors when `onError` is absent or itself
  throws or rejects. After a refused timer wake the next draw re-arms the
  session's deadline.

The driver passes `true` as the third argument to
`completeToolCall`. In terminal mode an invalid response or an oversized
provider diagnostic settles the pending call as a tool failure. The manual
two-argument form lets the embedder correct a refused outcome. A result
refused under the session limits appears in the query snapshot. When the
individual query errors exceed the result limit, the snapshot shows one
compact diagnostic and Retry can dispatch the call again. A result value
that JSON would lose or change in kind produces a tool error identifying the
value, in a function-map result and in MCP `structuredContent`: a function, a
non-finite number (boxed or not), a BigInt, a symbol as a value, a `Map` or
`Set`, an invalid `Date`, a typed array, an `ArrayBuffer` or a `DataView`.
Otherwise `JSON.stringify` is followed: an `undefined` property and a
symbol-keyed property are dropped, a valid `Date` becomes its ISO string, an
`undefined` array item or a hole becomes `null`, and an object's own `toJSON`
is used (ADR 0014). If delivery itself throws, the
driver reports the binding error and sends a terminal failure for a receipt
that is still pending. An admitted result survives a cut in dependent reader
evaluation. A second refusal that leaves the receipt pending
detaches the provider.

The driver samples `performance.now()` when installing intervals.
`sampleQueryClock` sets the clock for new registrations. Existing deadlines
run when the timer delivers `advanceQueryClock`. A wanted call's
`pending` getter reports whether the manager still owns the call.
When the manager releases a query call, a late completion can still advance
readers left at the round limit. The driver discards the payload without reading it.
Pause, detach, dispose and free revoke outstanding deliveries. A provider
owns its network request and any timeout. Its promise can settle later.

An action whose mutation a pause drops is reported once, with the kind
`QueriesPaused` and nothing more of its plan run, after resume either (#117).
A click while paused returns that failure in its own output, at the mutation
step; no call is made, the steps before it keep their effects, and `onError`
is not called (the caller has the output, as for any click). A plan whose
mutation call was pending when `pauseQueries()` ran comes back from the
pause's drain as an `onAction` output with that failure and no events, and
`onError` hears the same failure: an `Error` with `kind` `QueriesPaused` and
the reason as `message`. The reason says "not sent to the tool provider" when
the driver never took the call (the drain that would have taken it was
waiting for a task, so the provider is never invoked), and "handed to the tool
provider, so the tool may have run" when it had. A drain invokes the provider
for each call it takes while nothing has cancelled the round; a call taken by
the delivery before a free, dispose or detach is released without invoking
it, and so is one left after a pause or detach made inside that drain. The
session sees neither, so that reason stays conservative: the tool may have
run, not that it did. The texts are in
[runtime-binding.md](runtime-binding.md). The provider's promise settling later
changes nothing, and one that never settles is reported the same way. A
query `@Run` while paused starts no call and the plan carries on, as
upstream's does. A manual embedder finds the in-flight failure in the next
`takeQueryUpdate`.

After the app's `free()` or `dispose()` returns, the driver calls none of the
app's callbacks (`onError`, `onAction`, `onChange`), invokes no tool provider
and delivers no result into the session, for the session's current driver, a
closing driver and any driver detached earlier. Every action segment the
session produced before that is delivered exactly once, in order, before the
free returns (ADR 0014). So the free delivers what is left first: the rest of
a drain it was made inside, then what the session holds (segments a pause left
for a drain still waiting for a task), taking again while a callback it runs
produces more, up to three takes. Past that it stops, reports one
`LimitExceeded` through `onError` and frees; an app that resumes, clicks and
pauses from every callback would otherwise keep `free()` from returning.
Those callbacks run inside the app's own call, before it returns. An error
thrown by the session while taking an update during that delivery, and the
update's query evaluation error or reader-round cut, reach `onError` as a
drain would report them. A pause inside a callback no longer stops the
delivery of segments the drain already took; they are delivered after it.

The driver does not promise that JavaScript's own promise machinery leaves
alone values the app created: a job already queued may still read or run a
thenable's `then`, a promise subclass's constructor or `Symbol.species`, or a
getter on such a value after the free. Such a job can no longer call a
callback, invoke a provider or deliver a result. A provider's result and a
callback's return value are followed with `Promise.resolve`, as they always
were.

`setToolProvider(null)` (detach) first delivers every segment the same way,
while the driver is still attached. Then the detached driver's last drain
delivers whatever it takes, as it always did, and a free or dispose made
inside one of its callbacks finishes and stops it. Nothing is left in the
session for a later owner or a manual `takeQueryUpdate`.

A mutation step that fails while the program is unregistered reports the
registration's limit, which is also the session's evaluation error. For a
plan resumed after a call, `onError` therefore hears the same kind and message
twice: once as the evaluation error, when it first appears (repeats are not
reported), and once as that action's failure. They are two events: the
registration was refused, and this action stopped.

### Drawing a raw response

```typescript
interface RuntimeSession {
    setResponse(text: string | null, streaming: boolean): ResponseDraw;
}
```

Pass the whole response so far on the first draw and on edits. When the
session's prompt spec supplies a root component, the streaming parser uses it
to select the entry in programs that omit the `root` statement. Read the
returned `evaluation` as an ordinary `EvaluationOutput` and free it after
use. The same result record carries `name`, `attributes`, `context` and
`error`. A null response draws an empty program. A redraw with `evaluate()`
keeps the last raw response state. A successful `setProgram`, `replaceProgram`
or draw with an explicit response state returns control to the caller.

```js
const {evaluation, name, context, attributes, error} = session.setResponse(text, streaming);
try {
    // Disable buttons and inputs while the response is blocked, as upstream does.
    render(JSON.parse(evaluation.json), {disabled: evaluation.blocked, name, context, attributes, error});
} finally {
    evaluation.free();
}
```

The bundle reader strips the scripts section and supplies the session's gates
and Cloud script revision. The message's winning content marker supplies its
attributes. Context contains parsed JSON or the text received so far. An
invalid response after streaming returns the bundle error in `error` and the parser
diagnostic in `evaluation.parserErrorJson`. The program still draws.
Every draw's `evaluation.blocked` is the renderer's `blocked`, which upstream
hands its components as `isStreaming`: true while the response streams and,
for a response bundle, while it is incomplete or invalid. While the last draw
was a `setResponse`, a redraw with `evaluate()` reports it for that raw
response; after `evaluate(state)`, `setProgram` or `replaceProgram`, a plain
`evaluate()` is a settled draw of a bare program and reports false. A refused
update leaves it as it was. Clicks and Retry on a blocked response bundle are ignored
whatever the flag drew.

The entire raw response counts against the program-size limit, including
marker lines and Cloud script code. The reader uses the smaller of its
default size and depth ceilings and the session's configured ceilings.
Session limits also apply to the script names, revision and parsed program.
The package counts the error, the revision and the script names (and a byte
per name) in UTF-8 bytes against `maxJsonBytes` before the draw, as Rust
does again. It also refuses a response whose scripts JSON (the names as the
JSON array it hands to Rust, escapes included) exceeds `maxJsonBytes` bytes;
the native binding passes the names as a list, with no such text.
A refused edit preserves the displayed evaluation, its actions and pending
calls.

Streaming costs about what pushing the program does
([ADR 0015](adr/0015-an-incremental-reader-for-a-raw-response-that-grows.md)):
when the text continues the last one the session accepted, the package sends
only the new part across, and the session's reader goes through only the
lines that changed and parses only the new part of the program. Any other
edit is read afresh. Either way the result is the same as reading the whole
text, and a refused update leaves the reader at the last accepted text. [Built-package tests](../crates/openui-wasm/js/session-bundles.test.ts)
