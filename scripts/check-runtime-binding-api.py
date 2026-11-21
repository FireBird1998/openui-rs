#!/usr/bin/env python3
"""Check generated domain signatures; also called by swift-binding.sh."""
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent
methods = [
    "setQueryExtraValues", "attachToolProvider", "detachToolProvider",
    "takeQueryUpdate", "completeToolCall", "retryFailedQueries",
    "attachToolProviderAt", "resumeQueriesAt",
    "advanceQueryClock", "pauseQueries", "resumeQueries", "attachToolProviderWithMinimum",
    "actionAt", "dispose", "evaluate", "get", "getField", "initialize",
    "initializeField", "isPristine", "markBinding", "pendingContinuations",
    "registerFunction", "replaceProgram", "resume", "run", "runLegacy", "set", "setField", "setProgram",
    "setReferenceValues", "snapshot", "undecidedBindings", "version", "writeBinding",
]
# The native terminal completion; JavaScript passes completeToolCall's third argument.
native_methods = methods + ["evaluateWithResponse", "replaceProgramWithResponse", "finishToolCall"]

def camel(name):
    return re.sub(r"_([a-z])", lambda m: m[1].upper(), name)
native = (root / "crates/openui-ffi/src/runtime.rs").read_text()
wasm = (root / "crates/openui-wasm/src/runtime.rs").read_text()
inner = (root / "crates/openui-wasm/src/session.rs").read_text()
core = (root / "crates/openui-session/src/lib.rs").read_text()
core += (root / "crates/openui-session/src/queries.rs").read_text()
core += (root / "crates/openui-session/src/refresh.rs").read_text()
core += (root / "crates/openui-session/src/response.rs").read_text()
for source, label in [(native, "native"), (inner, "wasm inner"), (core, "session core")]:
    exported = {camel(n) for n in re.findall(r"pub fn (\w+)\(", source)}
    shared = set(native_methods) - {"dispose", "setReferenceValues"}
    if label == "session core":
        shared -= {"finishToolCall"}
        assert "pub fn finish_tool_call_json(" in (root / "crates/openui-session/src/tool_results.rs").read_text(), label
    assert shared <= exported, label
native_session = native.split("#[uniffi::export]", 1)[1].split("fn worker_loop", 1)[0]
assert {camel(n) for n in re.findall(r"pub fn (\w+)\(", native_session)} == set(native_methods) | {"new", "withLimits"}, "native session drift"
ffi_lib = (root / "crates/openui-ffi/src/lib.rs").read_text()
message_functions = {"parseMessage", "buildMessage"}
ffi_messages = (root / "crates/openui-ffi/src/message.rs").read_text()
wasm_messages = (root / "crates/openui-wasm/src/message.rs").read_text()
assert {camel(n) for n in re.findall(r"#\[uniffi::export\]\s*pub fn (\w+)\(", ffi_messages)} == message_functions
assert set(re.findall(r"js_name = (\w+)", wasm_messages)) == message_functions
reader_functions = message_functions | {"parseResponseBundle"}
ffi_bundle = (root / "crates/openui-ffi/src/response_bundle.rs").read_text()
wasm_bundle = (root / "crates/openui-wasm/src/response_bundle.rs").read_text()
assert re.search(r"#\[uniffi::export\]\s*pub fn parse_response_bundle\(", ffi_bundle)
assert "js_name = parseResponseBundle" in wasm_bundle
# The incremental raw-response reader (ADR 0015): the native stream session
# reads a raw response with its own parser; the WebAssembly session uses the
# same reader behind setResponse.
stream_session = ffi_lib.split("impl StreamSession {", 2)[2].split("\n}\n", 1)[0]
assert {camel(n) for n in re.findall(r"pub fn (\w+)\(", stream_session)} == {"new", "withLimits", "push", "pushAll", "set", "result", "finish", "readResponse"}, "native stream session drift"
assert re.findall(r"pub (\w+):", ffi_bundle.split("pub struct ResponseRead {", 1)[1].split("\n}", 1)[0]) == ["bundle", "message", "parse"]
def wasm_accessors(record_name):
    """JavaScript getter and setter names of a wasm record, from each method's
    own attribute: js_name if given, else the Rust name (setters drop `set_`)."""
    body = wasm.split("impl " + record_name + " {", 1)[1].split("\n}", 1)[0]
    getters, setters = set(), set()
    for attribute, function in re.findall(r"#\[wasm_bindgen\(([^\]]*)\)\]\s*pub fn (\w+)\(", body):
        options = [part.strip() for part in attribute.split(",")]
        js_name = next((o.split("=", 1)[1].strip() for o in options if o.startswith("js_name")), None)
        if "getter" in options:
            getters.add(js_name or camel(function))
        if "setter" in options:
            setters.add(js_name or camel(function.removeprefix("set_")))
    return getters, setters
for record_name, source in [("RuntimeLimits", native), ("ParseLimits", ffi_lib)]:
    native_field_order = re.findall(r"pub (\w+):", source.split("pub struct " + record_name + " {", 1)[1].split("\n}", 1)[0])
    if record_name == "RuntimeLimits":
        assert native_field_order == ["evaluation", "reference_preparation", "max_json_bytes", "max_continuations", "max_action_path_bytes", "max_cache_bytes"], "native limit constructor order drift"
        assert "#[uniffi(default = None)]" in source, "new native limit must default to absent"
    native_fields = set(native_field_order)
    core_fields = set(re.findall(r"pub (\w+):", core.split("pub struct " + record_name + " {", 1)[1].split("\n}", 1)[0]))
    assert native_fields == core_fields, record_name + " core/native drift"
    if record_name == "ParseLimits":
        assert "peak_memory" in native_fields, "missing opt-in memory field"
    getters, setters = wasm_accessors("ParseLimits" if record_name == "ParseLimits" else "RuntimeLimits")
    expected = {camel(field) for field in native_fields}
    assert getters == expected, f"{record_name} wasm getters {sorted(getters ^ expected)} drift"
    assert setters == expected, f"{record_name} wasm setters {sorted(setters ^ expected)} drift"
wasm_session = wasm.split("impl RuntimeSession {", 1)[1].split("pub struct RuntimeLimits", 1)[0]
assert set(re.findall(r"js_name = (\w+)", wasm_session)) == set(methods) | {"withLimits", "extractMcpToolResult", "sampleQueryClock", "__readResponseBundle", "__drawResponse"}, "wasm session drift"
assert "call: Arc<WantedToolCall>" in native and "outcome_json: String" in native
assert "call: &WantedToolCall" in wasm_session
assert "action: Option<ActionHandle>" in wasm_session
assert "evaluation: &EvaluationHandle" in wasm_session
assert "continuation: &ContinuationHandle" in wasm_session
assert "program: &ParsedProgram" in wasm_session
for name in ["InvalidToolCall", "ReferenceConflict", "ForeignHandle", "StaleHandle", "ConsumedHandle", "EvictedHandle", "InvalidHandle", "InvalidJson", "InvalidMarker", "RefusedWrite", "Disposed", "Evaluation", "LimitExceeded", "Worker", "QueriesPaused"]:
    assert name in core and name in wasm and name in (root / "crates/openui-ffi/src/lib.rs").read_text(), name
# Every kind Rust throws or reports is one the package guard rebuilds (ADR 0007).
guard = (root / "crates/openui-wasm/js/package-guards.js").read_text()
for kind in re.findall(r'=> "(\w+)",', wasm.split("pub(crate) fn kind(", 1)[1].split("\n}\n", 1)[0]):
    assert f"{kind}: true" in guard, ("package guard kind", kind)
    for doc in ["docs/wasm-runtime-binding-api.md", "docs/runtime-binding-api.md"]:
        assert kind in (root / doc).read_text(), (doc, kind)
assert "package-guards.js" in (root / "scripts/wasm.sh").read_text()
assert "#[uniffi::export(callback_interface)]" in native
assert "pub trait LibraryFunction: Send + Sync" in native
assert "arguments_json: String" in native and "Result<Option<String>, FunctionError>" in native
assert "callback: crate::callbacks::FunctionCallback" in wasm
callbacks = (root / "crates/openui-wasm/src/callbacks.rs").read_text()
callback_type = "(argumentsJson: string) => string | undefined"
assert callback_type in callbacks, "wasm callback JSON contract drift"
assert "dyn Fn(String) -> Result<Option<String>, String> + Send + Sync" in core, "core callback JSON contract drift"
assert "registerFunction: openuiSpec([openuiStr, openuiFunction])" in (root / "crates/openui-wasm/js/package-guards.js").read_text()
for doc in ["docs/runtime-binding-api.md", "docs/wasm-runtime-binding-api.md"]:
    assert "readerRoundsLimited" in (root / doc).read_text(), doc
    for name in (native_methods if doc == "docs/runtime-binding-api.md" else methods):
        assert name in (root / doc).read_text(), (doc, name)
driver_signature = "setToolProvider(provider?: ToolProvider | null, options?: ToolProviderOptions): void;"
response_signature = "setResponse(text: string | null, streaming: boolean): ResponseDraw;"
driver_api = (root / "docs/wasm-runtime-binding-api.md").read_text()
assert response_signature in driver_api
assert driver_signature in driver_api and "sampleQueryClock(now_ms: bigint): void;" in driver_api
print(f"Source: {len(native_methods)} native and {len(methods) + 3} wasm session methods, opaque capabilities, errors and limits checked")
if len(sys.argv) == 1:
    sys.exit(0)
if len(sys.argv) == 3 and sys.argv[1] == "--wasm":
    declarations = pathlib.Path(sys.argv[2]).read_text()
    for function in reader_functions:
        assert re.search(r"^export function " + function + r"\([^\n]*\): string;", declarations, re.M), function
    session = re.search(r"export class RuntimeSession\s*\{(.*?)\n\}", declarations, re.S)
    assert session, "missing wasm RuntimeSession"
    for method in methods + ["withLimits"]:
        assert re.search(r"\b" + method + r"\(", session[1]), method
    assert re.search(r"static extractMcpToolResult\(response_json: string, limits: ParseLimits\): string;", session[1]), "wasm MCP helper signature drift"
    assert response_signature in declarations
    assert driver_signature in declarations
    assert "sampleQueryClock(now_ms: bigint): void;" in session[1]
    assert "readonly pending: boolean;" in declarations
    assert "takeQueryUpdate(): QueryEffects;" in session[1]
    assert "completeToolCall(call: WantedToolCall, outcome_json: string, terminal?: boolean | null): void;" in session[1]
    assert "evaluate(response_state?: ResponseState): EvaluationOutput;" in session[1]
    assert re.search(r"replaceProgram\([^\n]+response_state\?: ResponseState\): ReplacementOutput;", session[1])
    assert "export interface ResponseState { streaming: boolean; isBundle?: boolean; complete?: boolean; error?: string | null; scripts?: string[]; scriptRevision?: string | null }" in declarations
    assert "setQueryExtraValues(tool_name: string, values_json: string): void;" in session[1]
    assert re.search(r"runLegacy\(json: string, message: string, form\?: string(?: \| null)?\): ActionOutput;", session[1])
    assert "readonly isQueryLoading: boolean;" in declarations
    assert "readonly blocked: boolean;" in declarations
    assert "readonly parserErrorJson: string | undefined;" in declarations
    assert re.search(r"run\(action: ActionHandle \| (?:null \| )?undefined, message: string, form\?: string(?: \| null)?\): ActionOutput;", session[1])
    for field in ["queryEvaluationError", "evaluationError"]:
        assert f"readonly {field}: {{ kind: string; message: string }} | null;" in declarations, field
    assert "readonly readerRoundsLimited: boolean;" in declarations
    assert "readonly nextRefreshDueMs: bigint | undefined;" in declarations
    assert "resumeQueriesAt(now_ms: bigint): void;" in session[1]
    assert "attachToolProviderAt(now_ms: bigint, minimum_ms: bigint): void;" in session[1]
    assert "advanceQueryClock(now_ms: bigint): void;" in session[1]
    assert "attachToolProviderWithMinimum(minimum_ms: bigint): void;" in session[1]
    assert "readonly actionCount: bigint;" in declarations
    assert "actionAt(index: bigint): ActionOutput | undefined;" in declarations
    assert "callAt(index: bigint): WantedToolCall | undefined;" in declarations
    assert "export type { QueryEffects, WantedToolCall };" in declarations
    for name in ["QueryEffects", "WantedToolCall"]:
        assert re.search(r"^declare class " + name + r" \{\s*private constructor\(\);", declarations, re.M), name
    assert callback_type in declarations, "wasm callback declaration drift"
    assert re.search(r"registerFunction\(name:\s*string,\s*callback:", session[1]), "wasm callback parameters drift"
    assert re.search(r"registerFunction\([^\n]*\): void;", session[1]), "wasm callback return drift"
    for name in ["ParsedProgram", "EvaluationHandle", "ActionHandle", "ContinuationHandle", "RuntimeLimits", "ParseLimits", "EvaluationOutput", "ActionOutput", "ReplacementOutput"]:
        assert "export class " + name in declarations, name
    # Each limit field is both readable and writable under its native name.
    for record_name, fields in [("RuntimeLimits", ["evaluation", "referencePreparation", "maxJsonBytes", "maxActionPathBytes", "maxContinuations", "maxCacheBytes"]),
                                ("ParseLimits", ["programSize", "resultSize", "resultFloor", "amplification", "resultCeiling", "depth", "work", "peakMemory"])]:
        body = re.search(r"export class " + record_name + r"\s*\{(.*?)\n\}", declarations, re.S)[1]
        for field in fields:
            plain = re.search(r"^\s*" + field + r":", body, re.M)
            assert plain or (re.search(r"^\s*get " + field + r"\(", body, re.M) and re.search(r"^\s*set " + field + r"\(", body, re.M)), record_name + "." + field
    print("wasm-bindgen declarations checked")
    sys.exit(0)
if len(sys.argv) not in (2, 3):
    sys.exit("usage: check-runtime-binding-api.py SWIFT_FILE [KOTLIN_FILE]")
swift = pathlib.Path(sys.argv[1]).read_text()
for function in reader_functions:
    assert re.search(r"^public func " + function + r"\([^\n]*\)\s*throws\s*->\s*String", swift, re.M), function
api = re.search(r"public protocol RuntimeSessionProtocol[^\{]*\{(.*?)\n\}", swift, re.S)
assert api, "missing Swift RuntimeSession protocol"
for method in native_methods:
    assert re.search(r"func\s+" + method + r"\(", api[1]), method
    if method != "dispose":
        assert re.search(r"func\s+" + method + r"\([^\n]*\)\s*throws", api[1]), method
for name, kind in [("action", "ActionHandle?"), ("continuation", "ContinuationHandle"),
                   ("evaluation", "EvaluationHandle"), ("program", "ParsedProgram")]:
    assert f"{name}: {kind}" in api[1]
assert re.search(r"func evaluate\(\) throws\s*-> EvaluationOutput", api[1])
assert re.search(r"func replaceProgram\(program: ParsedProgram, persisted: String\?, projected: String\?, nativeView: Bool\) throws\s*-> ReplacementOutput", api[1])
assert re.search(r"func takeQueryUpdate\(\) throws\s*-> QueryEffects", api[1])
assert re.search(r"func completeToolCall\(call: WantedToolCall, outcomeJson: String\) throws[ \t]*$", api[1], re.M)
assert re.search(r"func finishToolCall\(call: WantedToolCall, outcomeJson: String\) throws[ \t]*$", api[1], re.M)
assert "querySnapshotJson: String? = nil, queryEvaluationError: OpenUiError? = nil" in swift
assert "public init(streaming: Bool = false, isBundle: Bool = false, complete: Bool = true, error: String? = nil, scripts: [String] = [], scriptRevision: String? = nil)" in swift
assert re.search(r"func evaluateWithResponse\(responseState: ResponseState\) throws\s*-> EvaluationOutput", api[1])
assert re.search(r"func replaceProgramWithResponse\([^\n]*responseState: ResponseState\) throws\s*-> ReplacementOutput", api[1])
assert re.search(r"func run\(action: ActionHandle\?, message: String, form: String\?\) throws\s*-> ActionOutput", api[1])
assert re.search(r"func runLegacy\(json: String, message: String, form: String\?\) throws\s*-> ActionOutput", api[1])
assert "func setQueryExtraValues(toolName: String, valuesJson: String) throws" in api[1]
assert "isQueryLoading: Bool = false, parserErrorJson: String? = nil" in swift
assert re.search(r"public var blocked: Bool\b", swift) and "blocked: Bool = false" in swift, "Swift blocked flag drift"
assert "nextRefreshDueMs: UInt64? = nil" in swift
assert "func resumeQueriesAt(nowMs: UInt64) throws" in api[1]
assert "func attachToolProviderAt(nowMs: UInt64, minimumMs: UInt64) throws" in api[1]
assert "func advanceQueryClock(nowMs: UInt64) throws" in api[1]
assert "func attachToolProviderWithMinimum(minimumMs: UInt64) throws" in api[1]
assert "actions: [ActionOutput] = []" in swift
assert "public var calls: [WantedToolCall]" in swift
assert "evaluationError: OpenUiError?, actions: [ActionOutput] = [], nextRefreshDueMs: UInt64? = nil, readerRoundsLimited: Bool = false)" in swift
assert "case ReferenceConflict" in swift
assert "public var referencePreparation: ParseLimits" in swift
assert "maxActionPathBytes: UInt64? = nil" in swift, "new Swift limit must stay opt-in"
assert "maxCacheBytes: UInt64? = nil" in swift, "the Swift cache cap must stay opt-in"
assert "peakMemory: UInt64? = nil" in swift, "memory limit must stay optional in Swift"
assert re.search(r"public\s+protocol LibraryFunction", swift)
assert re.search(r"func\s+call\(argumentsJson: String\)\s*throws\s*->\s*String\?", swift), "Swift callback signature drift"
assert "case Thrown(reason: String" in swift
assert re.search(r"public func extractMcpToolResult\(responseJson: String, limits: ParseLimits\)\s*throws\s*->\s*String", swift), "Swift MCP helper signature drift"
assert "public var maxPlans:" not in swift
assert "compile_fail" not in swift, "rustdoc-only snippet leaked to Swift"
for error in ["ForeignHandle", "ConsumedHandle", "EvictedHandle", "Evaluation", "QueriesPaused"]:
    assert f"case {error}" in swift
if len(sys.argv) == 3:
    kotlin = pathlib.Path(sys.argv[2]).read_text()
    for function in reader_functions:
        assert re.search(r"fun `" + function + r"`\([^\n]*\): kotlin.String", kotlin), function
    api = re.search(r"public interface RuntimeSessionInterface\s*\{(.*?)\n\}", kotlin, re.S)
    assert api, "missing Kotlin RuntimeSession interface"
    for method in native_methods:
        assert f"`{method}`(" in api[1], method
    for name, kind in [("action", "ActionHandle?"), ("continuation", "ContinuationHandle"),
                       ("evaluation", "EvaluationHandle"), ("program", "ParsedProgram")]:
        assert f"`{name}`: {kind}" in api[1]
    assert "fun `evaluate`(): EvaluationOutput" in api[1]
    assert "fun `replaceProgram`(`program`: ParsedProgram, `persisted`: kotlin.String?, `projected`: kotlin.String?, `nativeView`: kotlin.Boolean): ReplacementOutput" in api[1]
    assert "fun `takeQueryUpdate`(): QueryEffects" in api[1]
    assert re.search(r"fun `completeToolCall`\(`call`: WantedToolCall, `outcomeJson`: kotlin.String\)[ \t]*$", api[1], re.M)
    assert re.search(r"fun `finishToolCall`\(`call`: WantedToolCall, `outcomeJson`: kotlin.String\)[ \t]*$", api[1], re.M)
    assert "`queryEvaluationError`: OpenUiException? = null" in kotlin and "`querySnapshotJson`: kotlin.String? = null" in kotlin
    for declaration in ["`streaming`: kotlin.Boolean = false", "`complete`: kotlin.Boolean = true",
                        "`isBundle`: kotlin.Boolean = false", "`scripts`: List<kotlin.String> = listOf()",
                        "`scriptRevision`: kotlin.String? = null", "`error`: kotlin.String? = null",
                        "`isQueryLoading`: kotlin.Boolean = false", "`parserErrorJson`: kotlin.String? = null",
                        "`blocked`: kotlin.Boolean = false"]:
        assert declaration in kotlin, declaration
    assert "fun `setQueryExtraValues`(`toolName`: kotlin.String, `valuesJson`: kotlin.String)" in api[1]
    assert "fun `runLegacy`(`json`: kotlin.String, `message`: kotlin.String, `form`: kotlin.String?): ActionOutput" in api[1]
    assert "fun `run`(`action`: ActionHandle?, `message`: kotlin.String, `form`: kotlin.String?): ActionOutput" in api[1]
    assert "fun `evaluateWithResponse`(`responseState`: ResponseState): EvaluationOutput" in api[1]
    assert "`responseState`: ResponseState): ReplacementOutput" in api[1]
    assert "`readerRoundsLimited`: kotlin.Boolean = false" in kotlin
    assert "`nextRefreshDueMs`: kotlin.ULong? = null" in kotlin
    assert "fun `resumeQueriesAt`(`nowMs`: kotlin.ULong)" in api[1]
    assert "fun `attachToolProviderAt`(`nowMs`: kotlin.ULong, `minimumMs`: kotlin.ULong)" in api[1]
    assert "fun `advanceQueryClock`(`nowMs`: kotlin.ULong)" in api[1]
    assert "fun `attachToolProviderWithMinimum`(`minimumMs`: kotlin.ULong)" in api[1]
    assert "`actions`: List<ActionOutput> = listOf()" in kotlin
    effects = kotlin.split("data class QueryEffects (", 1)[1].split("):", 1)[0]
    assert re.findall(r"var `(\w+)`:", effects) == ["changed", "calls", "evaluationError", "actions", "nextRefreshDueMs", "readerRoundsLimited"], "QueryEffects positional constructor drift"
    assert "class ReferenceConflict" in kotlin
    assert "interface LibraryFunction" in kotlin
    assert "fun `call`(`argumentsJson`: kotlin.String): kotlin.String?" in kotlin, "Kotlin callback signature drift"
    assert "sealed class FunctionException" in kotlin
    assert "@Throws(OpenUiException::class)" in kotlin
    assert "`referencePreparation`: ParseLimits" in kotlin
    assert "`maxActionPathBytes`: kotlin.ULong? = null" in kotlin, "new Kotlin limit must stay opt-in"
    assert "`maxCacheBytes`: kotlin.ULong? = null" in kotlin, "the Kotlin cache cap must stay opt-in"
    assert "`peakMemory`: kotlin.ULong? = null" in kotlin, "memory limit must stay optional in Kotlin"
    assert re.search(r"fun `extractMcpToolResult`\(`responseJson`: kotlin.String, `limits`: ParseLimits\): kotlin.String", kotlin), "Kotlin MCP helper signature drift"
    assert "`maxPlans`:" not in kotlin
    assert "onMalformedInput(CodingErrorAction.REPORT)" in kotlin
    assert "encode(CharBuffer.wrap(value))" in kotlin
    assert "compile_fail" not in kotlin
    for error in ["ForeignHandle", "ConsumedHandle", "EvictedHandle", "Evaluation", "QueriesPaused"]:
        assert f"class {error}" in kotlin
print(f"{'Swift/Kotlin' if len(sys.argv) == 3 else 'Swift'}: {len(native_methods)} session methods, opaque objects, typed errors and current limits checked")
