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
session.actionAt(evaluation: EvaluationHandle, path: string[]): ActionHandle
session.dispose(): void
session.evaluate(): EvaluationOutput
session.get(name: string): string | undefined
session.getField(form: string | undefined, name: string): string | undefined
session.initialize(persisted?: string): InitializationOutput
session.initializeField(form: string | undefined, componentType: string | undefined, name: string, json: string | undefined, streaming: boolean): FieldWriteOutput | null
session.isPristine(name: string): boolean
session.markBinding(component: string, prop: string): void
session.registerFunction(name: string, callback: (argumentsJson: string) => string | undefined): void
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

`new ParseLimits()` has optional bigint properties programSize, resultSize,
resultFloor, amplification, resultCeiling, depth, work; defaults are undefined.
`new RuntimeLimits()` has evaluation and referencePreparation (ParseLimits),
maxJsonBytes (8388608n), maxContinuations (64n), optional maxActionPathBytes
(undefined). Set e.g. 4096n for a smaller opt-in retained-path cap. referencePreparation.depth is
32768n; all other ParseLimits fields default to undefined. Limit getters return
copies: edit one, assign it back, then free the copy. Limits are fixed when a
session is constructed. `ActionHandle.retain()` returns another opaque token;
the package uses this to borrow an optional action without consuming the caller's
object (wasm-bindgen cannot lower Option<&ActionHandle>).

```typescript
interface EvaluationOutput {
  readonly json: string; readonly sizeLimited: boolean; readonly depthLimited: boolean;
  readonly evaluation: EvaluationHandle;
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
EvictedHandle, InvalidHandle, InvalidMarker, InvalidJson, RefusedWrite,
Evaluation, LimitExceeded, Worker, InvalidSchema, InvalidPromptSpec, Prompt.
Failures inside actions and initialization appear in the records above, with
that same kind/message shape. Reentry uses Worker("session already in use")
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
streaming/pristine rules, host events, bundleBlocked responsibilities and state
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
