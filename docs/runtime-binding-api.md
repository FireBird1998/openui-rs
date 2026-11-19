# Runtime binding application interface

Taken from UniFFI 0.32.2 output. Generated converters, unsafe/testing
constructors and native-object cleanup machinery are omitted. Capabilities
are objects: ParsedProgram, EvaluationHandle, ActionHandle, ContinuationHandle and WantedToolCall.
They have no application constructor or numeric slot. See [session semantics
and limits](runtime-binding.md).

Apps with an MCP client can call the standalone
`extractMcpToolResult(responseJson: String, limits: ParseLimits)` function in
Swift or Kotlin. It returns JSON text with a `value` field or an `error` field
containing the tool's raw error message. Malformed JSON, a response that
would throw a JavaScript TypeError, and a limit refusal raise `InvalidJson`,
`Evaluation`, and `LimitExceeded`, respectively. The helper applies
`resultSize` (fixed result ceiling), `work`, and `depth` before producing
the outcome. See the Rust runtime's `extract_mcp_tool_result` for details.

Swift (`import OpenUIFFI`); fallible calls throw OpenUiError:

```swift
convenience init(library: ComponentLibrary, promptSpec: String?) throws
static func withLimits(library: ComponentLibrary, promptSpec: String?, limits: RuntimeLimits) throws -> RuntimeSession
func attachToolProvider() throws
func detachToolProvider() throws
func evaluateWithResponse(responseState: ResponseState) throws -> EvaluationOutput
func replaceProgramWithResponse(program: ParsedProgram, persisted: String?, projected: String?, nativeView: Bool, responseState: ResponseState) throws -> ReplacementOutput
func completeToolCall(call: WantedToolCall, outcomeJson: String) throws
func takeQueryUpdate() throws -> QueryEffects
func actionAt(evaluation: EvaluationHandle, path: [String]) throws -> ActionHandle
func dispose()
func evaluate() throws -> EvaluationOutput
func get(name: String) throws -> String?
func getField(form: String?, name: String) throws -> String?
func initialize(persisted: String?) throws -> InitializationOutput
func initializeField(form: String?, componentType: String?, name: String, json: String?, streaming: Bool) throws -> FieldWriteOutput?
func isPristine(name: String) throws -> Bool
func markBinding(component: String, prop: String) throws
func registerFunction(name: String, callback: LibraryFunction) throws
func replaceProgram(program: ParsedProgram, persisted: String?, projected: String?, nativeView: Bool) throws -> ReplacementOutput
func pendingContinuations() throws -> UInt64
func resume(continuation: ContinuationHandle, succeeded: Bool) throws -> ActionOutput
func run(action: ActionHandle?, message: String, form: String?) throws -> ActionOutput
func runLegacy(json: String, message: String, form: String?) throws -> ActionOutput
func set(name: String, json: String?) throws -> Bool
func setField(form: String?, componentType: String?, name: String, json: String?, save: Bool) throws -> FieldWriteOutput
func setProgram(program: ParsedProgram) throws
func setReferenceValues(json: String) throws
func snapshot() throws -> String
func undecidedBindings() throws -> [BindingPair]
func version() throws -> UInt64
func writeBinding(marker: String, json: String) throws -> Bool
```

Kotlin (`import uniffi.openui_ffi.*`); constructors and fallible methods
carry @Throws(OpenUiException::class), while dispose is infallible:

```kotlin
constructor(library: ComponentLibrary, promptSpec: String?)
// companion object:
fun withLimits(library: ComponentLibrary, promptSpec: String?, limits: RuntimeLimits): RuntimeSession
fun attachToolProvider()
fun detachToolProvider()
fun evaluateWithResponse(responseState: ResponseState): EvaluationOutput
fun replaceProgramWithResponse(program: ParsedProgram, persisted: String?, projected: String?, nativeView: Boolean, responseState: ResponseState): ReplacementOutput
fun completeToolCall(call: WantedToolCall, outcomeJson: String)
fun takeQueryUpdate(): QueryEffects
fun actionAt(evaluation: EvaluationHandle, path: List<kotlin.String>): ActionHandle
fun dispose()
fun evaluate(): EvaluationOutput
fun get(name: kotlin.String): kotlin.String?
fun getField(form: kotlin.String?, name: kotlin.String): kotlin.String?
fun initialize(persisted: kotlin.String?): InitializationOutput
fun initializeField(form: kotlin.String?, componentType: kotlin.String?, name: kotlin.String, json: kotlin.String?, streaming: kotlin.Boolean): FieldWriteOutput?
fun isPristine(name: kotlin.String): kotlin.Boolean
fun markBinding(component: kotlin.String, prop: kotlin.String)
fun registerFunction(name: kotlin.String, callback: LibraryFunction)
fun replaceProgram(program: ParsedProgram, persisted: kotlin.String?, projected: kotlin.String?, nativeView: kotlin.Boolean): ReplacementOutput
fun pendingContinuations(): kotlin.ULong
fun resume(continuation: ContinuationHandle, succeeded: kotlin.Boolean): ActionOutput
fun run(action: ActionHandle?, message: kotlin.String, form: kotlin.String?): ActionOutput
fun runLegacy(json: kotlin.String, message: kotlin.String, form: kotlin.String?): ActionOutput
fun set(name: kotlin.String, json: kotlin.String?): kotlin.Boolean
fun setField(form: kotlin.String?, componentType: kotlin.String?, name: kotlin.String, json: kotlin.String?, save: kotlin.Boolean): FieldWriteOutput
fun setProgram(program: ParsedProgram)
fun setReferenceValues(json: kotlin.String)
fun snapshot(): kotlin.String
fun undecidedBindings(): List<BindingPair>
fun version(): kotlin.ULong
fun writeBinding(marker: kotlin.String, json: kotlin.String): kotlin.Boolean
```

Records (Swift field types; Kotlin maps Bool to Boolean, UInt64 to ULong,
arrays to List, OpenUiError to OpenUiException, and keeps record/object names):

```swift
struct RuntimeLimits {
    public var evaluation: ParseLimits
    public var referencePreparation: ParseLimits
    public var maxJsonBytes: UInt64
    public var maxContinuations: UInt64
    public var maxActionPathBytes: UInt64?
}
struct BindingPair {
    public var component: String
    public var prop: String
}
struct EvaluationOutput {
    public var json: String
    public var sizeLimited: Bool
    public var depthLimited: Bool
    public var evaluation: EvaluationHandle
    public var querySnapshotJson: String? // defaults to nil
    public var queryEvaluationError: OpenUiError? // defaults to nil
}
struct QueryEffects {
    public var changed: Bool
    public var calls: [WantedToolCall]
    public var evaluationError: OpenUiError?
}
struct ResponseState {
    public var streaming: Bool // defaults to false
}
struct ReplacementOutput {
    public var initialization: InitializationOutput
    public var projection: InitializationOutput
    public var evaluation: EvaluationOutput
    public var snapshot: String
    public var version: UInt64
}
struct InitializationOutput {
    public var changed: Bool
    public var refused: [StateRefusal]
}
struct StateRefusal {
    public var name: String
    public var error: OpenUiError
}
struct FieldWriteOutput {
    public var changed: Bool
    public var saveRequested: Bool
}
struct ActionOutput {
    public var events: [String]
    public var failure: ActionFailureOutput?
    public var sizeLimited: Bool
    public var depthLimited: Bool
    public var continuation: ContinuationHandle?
}
struct ActionFailureOutput {
    public var step: UInt64?
    public var error: OpenUiError
}
struct ParseOutput {
    public var program: ParsedProgram
    public var json: String
    public var depthLimited: Bool
    public var sizeLimited: Bool
    public var parseNanos: UInt64
    public var jsonNanos: UInt64
}
```

Swift error cases / Kotlin OpenUiException subclasses: Disposed, ForeignHandle,
StaleHandle, ConsumedHandle, EvictedHandle, InvalidHandle, InvalidMarker, InvalidToolCall, ReferenceConflict; InvalidJson(reason),
RefusedWrite(reason), Evaluation(reason), LimitExceeded(reason), Worker(reason), and existing
InvalidSchema(reason), InvalidPromptSpec(reason), Prompt(reason). Reasons are
String. ActionOutput.failure.error and StateRefusal.error use the same typed error.
ParseLimits has eight optional UInt64/ULong fields: programSize, resultSize,
resultFloor, amplification, resultCeiling, depth, work, peakMemory; all default to absent.
peakMemory counts portable evaluation reservations (ADR 0011), not process heap.
RuntimeSession's default constructor uses RuntimeLimits defaults; withLimits
takes a record containing the four existing fields and optional maxActionPathBytes.

State admission uses the existing `RuntimeLimits.evaluation` result-size ceiling
(`resultCeiling`, or `resultSize` which sets both floor and ceiling); default
8 MiB. It is independent of drawing's scaled room and referencePreparation.
No separate field, method or error variant was added. All Store refusals use
RefusedWrite(reason), whose reason includes the configured byte cap. Initialize
reports each refused persisted/default entry in InitializationOutput.refused and
can accept other entries in the same batch. See [state admission](runtime-binding.md)
and [ADR 0005](adr/0005-admit-total-state-incrementally.md); native session
ownership remains [ADR 0006](adr/0006-native-runtime-sessions-own-a-worker-and-opaque-capabilities.md).


Swift `public protocol LibraryFunction: AnyObject, Sendable` declares
`func call(argumentsJson: String) throws -> String?`. Kotlin `LibraryFunction`
declares `fun call(argumentsJson: String): String?`. Throw Swift
`FunctionError.Thrown(reason: String)` / Kotlin `FunctionException.Thrown(reason)`
for the runtime's exact thrown-message diagnostic. registerFunction itself
throws OpenUiError / OpenUiException on registration refusal. Names must be
declared; at most 256 names and maxJsonBytes of names plus 64 bytes per entry
are retained. Native calls run on the session worker without a timeout. Same-session calls
on that worker or inside a host destructor refuse; reentrant dispose is an
infallible no-op. Other threads queue normally, and their dispose waits for the
callback to return. Host code must not wait for another thread to call this same
session. See the
[callback contract and limitations](runtime-binding.md#host-library-functions).
`maxActionPathBytes` defaults to nil/null; set 4096 for a smaller opt-in path cap.

The new maxActionPathBytes constructor parameter is last and defaults to nil/null
in generated Swift/Kotlin; existing four-argument initializers keep working.

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

Call `takeQueryUpdate()` after session operations to claim wanted calls and the
redraw flag. Read each call's `toolName()` and optional `argumentsJson()` and
pass the object back to `completeToolCall`. A refused draw preserves the
previous root and evaluation handle. On a blocked first draw, a query with
a provider reads as null. Without a provider it reads its declared default,
as in the upstream renderer. [Driving queries](runtime-binding.md#driving-queries)
