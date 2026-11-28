#!/usr/bin/env python3
"""Check binding surfaces. Use --self-test to exercise the InvalidAttribute checks."""
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
# same reader behind setResponse. A native driver can also hand it only what
# was appended and read the accepted response again to draw (#206).
stream_session = ffi_lib.split("impl StreamSession {", 2)[2].split("\n}\n", 1)[0]
assert {camel(n) for n in re.findall(r"pub fn (\w+)\(", stream_session)} == {"new", "withLimits", "push", "pushAll", "set", "result", "finish", "readResponse", "appendResponse", "rereadResponse"}, "native stream session drift"
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
assert set(re.findall(r"js_name = (\w+)", wasm_session)) == set(methods) | {"withLimits", "sampleQueryClock", "__readResponseBundle", "__drawResponse"}, "wasm session drift"
# The MCP result helper is a free function in both bindings, as the other
# stateless helpers are (#120), and the package guard wraps it as one.
assert re.search(r"#\[uniffi::export\]\s*pub fn extract_mcp_tool_result\(", ffi_lib), "native MCP helper must be a free function"
assert re.search(r"#\[wasm_bindgen\(js_name = extractMcpToolResult\)\]\s*pub fn extract_mcp_tool_result\(\s*response_json: &str,", wasm), "wasm MCP helper must be a free function"
assert "extractMcpToolResult" not in wasm_session, "wasm MCP helper must not be a session static"
mcp_guard = (root / "crates/openui-wasm/js/package-guards.js").read_text()
assert re.search(r"^extractMcpToolResult = \(\.\.\.args\) =>", mcp_guard, re.M), "package guard must wrap the free MCP helper"
assert "extractMcpToolResult: openuiSpec" not in mcp_guard, "package guard still describes the MCP helper as a static"
assert re.search(r"parseResponseBundle, extractMcpToolResult \};", mcp_guard), "package guard must export the MCP helper"
# The helper's outcome kind for an error the tool reported is ToolReported;
# ToolError was its old name and meant something else in the query manager.
mcp_core = (root / "crates/openui-runtime/src/mcp.rs").read_text()
mcp_session = (root / "crates/openui-session/src/mcp.rs").read_text()
assert re.search(r"^    ToolReported \{", mcp_core, re.M), "MCP helper outcome kind ToolReported missing"
assert "McpToolOutcome::ToolReported" in mcp_session, "session MCP transport drift"
for label, source in [("runtime", mcp_core), ("session", mcp_session)]:
    assert "ToolError" not in source, ("old MCP helper outcome kind", label)
# A message attribute that cannot be URI-encoded has its own kind (#120).
def check_message_attribute(source, label):
    assert re.search(r"MessageError::InvalidAttribute => OpenUiError::InvalidAttribute \{ reason \}", source), ("attribute kind", label)
    assert not re.search(r"MessageError::InvalidAttribute\s*\|", source), ("attribute kind shares a kind", label)

for label, source in [("native", ffi_messages), ("wasm", wasm_messages)]:
    check_message_attribute(source, label)

def check_native_session_attribute(source):
    assert re.search(r"openui_session::OpenUiError::InvalidAttribute \{ reason \} =>\s*"
                     r"(?:\{\s*)?Self::InvalidAttribute \{ reason \}", source), "native session InvalidAttribute conversion"

check_native_session_attribute(native)
assert "call: Arc<WantedToolCall>" in native and "outcome_json: String" in native
assert "call: &WantedToolCall" in wasm_session
assert "action: Option<ActionHandle>" in wasm_session
assert "evaluation: &EvaluationHandle" in wasm_session
assert "continuation: &ContinuationHandle" in wasm_session
assert "program: &ParsedProgram" in wasm_session
for name in ["InvalidToolCall", "ReferenceConflict", "ForeignHandle", "StaleHandle", "ConsumedHandle", "EvictedHandle", "InvalidHandle", "InvalidJson", "InvalidMarker", "RefusedWrite", "Disposed", "Evaluation", "LimitExceeded", "Worker", "QueriesPaused", "InvalidAttribute"]:
    assert name in core and name in wasm and name in (root / "crates/openui-ffi/src/lib.rs").read_text(), name
# Every kind Rust throws or reports is one the package guard rebuilds (ADR 0007).
guard = (root / "crates/openui-wasm/js/package-guards.js").read_text()
wasm_kind = wasm.split("pub(crate) fn kind(", 1)[1].split("\n}\n", 1)[0]

def check_wasm_attribute_kind(source):
    assert 'core::OpenUiError::InvalidAttribute { .. } => "InvalidAttribute",' in source, "wasm InvalidAttribute kind"

check_wasm_attribute_kind(wasm_kind)
for kind in re.findall(r'=> "(\w+)",', wasm_kind):
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
# A library definition (#137): one shared policy in the session crate, the
# same entry points and text accessors in both bindings, guarded and
# documented, and refusals of existing kinds only.
session_definition = (root / "crates/openui-session/src/library_definition.rs").read_text()
assert re.search(r"pub fn define_library\(\s*definition: &str,\s*limits: &RuntimeLimits,", session_definition), "shared library definition entry point drift"
assert "admit_text(definition, limits.max_json_bytes)?;" in session_definition, "a definition must pass the host input limit before it is read"
assert session_definition.index("admit_text(") < session_definition.index("parse_json_with_limits("), "the limit must come before reading"
for accessor in ["json_schema = stringify_json(&schema)", "prompt_spec = stringify_json(&spec)"]:
    assert accessor in session_definition, ("definition text must be stringify_json's", accessor)
native_library = ffi_lib.split("impl ComponentLibrary {", 1)[1].split("\n}\n", 1)[0]
assert {camel(n) for n in re.findall(r"pub fn (\w+)\(", native_library)} == {"new", "fromSchemaWithLimits", "fromDefinition", "fromDefinitionWithLimits", "componentNames", "jsonSchema", "promptSpec", "prompt", "cloudConfig"}, "native ComponentLibrary drift"
assert re.search(r"#\[uniffi::constructor\]\s*pub fn from_definition\(definition: String\) -> Result<Arc<Self>, OpenUiError>", native_library)
assert re.search(r"#\[uniffi::constructor\]\s*pub fn from_definition_with_limits\(\s*definition: String,\s*limits: RuntimeLimits,\s*\) -> Result<Arc<Self>, OpenUiError>", native_library)
for accessor in ["json_schema", "prompt_spec"]:
    assert f"pub fn {accessor}(&self) -> Option<String>" in native_library, accessor
# A defined library's prompt and cloud config (#135): one shared policy in
# the session crate, options and instructions held to the host input limit
# before they are read, the same two methods in both bindings.
assert "pub fn prompt(&self, options: Option<String>) -> Result<Option<String>, OpenUiError>" in native_library, "native prompt drift"
assert re.search(r"pub fn cloud_config\(\s*&self,\s*options: Option<String>,\s*instructions: Option<String>,\s*\) -> Result<Option<String>, OpenUiError>", native_library), "native cloudConfig drift"
assert re.search(r"pub fn prompt\(&self, options: Option<&str>\) -> Result<Option<String>, OpenUiError>", session_definition), "shared prompt drift"
assert re.search(r"pub fn cloud_config\(\s*&self,\s*options: Option<&str>,\s*instructions: Option<&str>,\s*\) -> Result<Option<String>, OpenUiError>", session_definition), "shared cloud config drift"
prompt_options = session_definition.split("fn prompt_options(", 1)[1]
assert prompt_options.index("admit_text(options, limits.max_json_bytes)?;") < prompt_options.index("parse_json_with_limits("), "prompt options must pass the host input limit before they are read"
assert "admit_text(instructions, defined.limits.max_json_bytes)?;" in session_definition, "instructions must pass the host input limit"
wasm_lib = (root / "crates/openui-wasm/src/lib.rs").read_text()
wasm_library = wasm_lib.split("impl ComponentLibrary {", 1)[1].split("\n}\n", 1)[0]
assert set(re.findall(r"js_name = (\w+)", wasm_library)) == {"fromSchemaWithLimits", "fromDefinition", "fromDefinitionWithLimits", "jsonSchema", "promptSpec", "cloudConfig"}, "wasm ComponentLibrary drift"
assert "pub fn prompt(&self, options: Option<String>) -> Result<Option<String>, JsValue>" in wasm_library, "wasm prompt drift"
assert "self.0.prompt(options.as_deref())" in wasm_library and "self.0\n            .cloud_config(options.as_deref(), instructions.as_deref())" in wasm_library, "wasm prompts must use the shared policy"
assert "limits: &runtime::RuntimeLimits" in wasm_library
for label, source in [("native", ffi_lib), ("wasm", wasm_lib)]:
    assert "openui_session::define_library(" in source, ("bindings share the definition policy", label)
library_guard = (root / "crates/openui-wasm/js/package-guards.js").read_text()
assert "fromDefinition: openuiSpec([openuiStr], { check: openuiAdmitDefinition })," in library_guard
assert "fromSchemaWithLimits: openuiSpec([openuiStr, openuiCapabilityOf(RuntimeLimits)])," in library_guard
assert ("fromDefinitionWithLimits: openuiSpec([openuiStr, openuiCapabilityOf(RuntimeLimits)], "
        "{ check: openuiAdmitDefinition }),") in library_guard
assert "methods: { free: openuiFree, jsonSchema: openuiNone, promptSpec: openuiNone,\n      prompt: openuiSpec([openuiOptStr]), cloudConfig: openuiSpec([openuiOptStr, openuiOptStr]) }," in library_guard
# A library's JSON Schema (#140): host input held to the same limit, before
# it is read, by one shared policy; the plain constructor keeps the defaults.
assert re.search(r"pub fn library_from_schema\(\s*schema: &str,\s*limits: &RuntimeLimits,", session_definition), "shared schema entry point drift"
assert "admit_text(schema, limits.max_json_bytes)?;" in session_definition, "a schema must pass the host input limit before it is read"
assert session_definition.index("admit_text(schema") < session_definition.index("read_host_json(schema)"), "the schema limit must come before reading"
assert re.search(r"#\[uniffi::constructor\]\s*pub fn new\(schema: String\) -> Result<Arc<Self>, OpenUiError> \{\s*Self::from_schema_with_limits\(schema, RuntimeLimits::default\(\)\)", native_library), "native new must apply the default limits"
assert re.search(r"#\[uniffi::constructor\]\s*pub fn from_schema_with_limits\(\s*schema: String,\s*limits: RuntimeLimits,\s*\) -> Result<Arc<Self>, OpenUiError>", native_library)
assert "Self::from_schema(schema, &openui_session::RuntimeLimits::default())" in wasm_library, "wasm constructor must apply the default limits"
for label, source in [("native", ffi_lib), ("wasm", wasm_lib)]:
    assert "openui_session::library_from_schema(" in source, ("bindings share the schema policy", label)
    assert "Library::from_json_schema(" not in source.split("impl ComponentLibrary {", 1)[1].split("\n}\n", 1)[0], ("no unlimited schema path", label)
for doc, names in [("docs/runtime-binding-api.md", ["fromSchemaWithLimits", "fromDefinition", "fromDefinitionWithLimits", "jsonSchema", "promptSpec", "maxJsonBytes", "prompt(options", "cloudConfig"]),
                   ("docs/wasm-runtime-binding-api.md", ["fromSchemaWithLimits", "fromDefinition", "fromDefinitionWithLimits", "jsonSchema", "promptSpec", "maxJsonBytes", "prompt(options", "cloudConfig"]),
                   ("docs/library-definition.md", ["host input byte limit", "LimitExceeded", "InvalidJson", "jsonSchema", "promptSpec", "prompt(options", "cloudConfig", "InvalidPromptSpec"])]:
    text = (root / doc).read_text()
    for name in names:
        # The whole name: fromDefinitionWithLimits must not stand in for
        # fromDefinition.
        assert re.search(r"(?<!\w)" + re.escape(name) + r"(?!\w)", text), (doc, name)
print(f"Source: {len(native_methods)} native and {len(methods) + 3} wasm session methods, library schemas and definitions, opaque capabilities, errors and limits checked")

error_cases = ["ReferenceConflict", "ForeignHandle", "ConsumedHandle", "EvictedHandle",
               "Evaluation", "QueriesPaused", "InvalidAttribute"]

def check_error_case(source, declaration, error):
    assert re.search(rf"\b{declaration} {error}\b", source), (declaration, error)

if sys.argv[1:2] == ["--self-test"]:
    import unittest

    class AttributeKindTests(unittest.TestCase):
        def assert_rejects_drift(self, check, source, before, after):
            check(source)
            self.assertEqual(source.count(before), 1, before)
            with self.assertRaises(AssertionError):
                check(source.replace(before, after))

        def test_wasm_error_kind(self):
            self.assert_rejects_drift(check_wasm_attribute_kind, wasm_kind,
                                      '=> "InvalidAttribute"', '=> "InvalidJson"')

        def test_native_session_error_conversion(self):
            self.assert_rejects_drift(check_native_session_attribute, native,
                                      "Self::InvalidAttribute { reason }", "Self::InvalidJson { reason }")

        def test_native_message_error(self):
            self.assert_rejects_drift(lambda s: check_message_attribute(s, "native"), ffi_messages,
                                      "OpenUiError::InvalidAttribute", "OpenUiError::InvalidJson")

        def test_wasm_message_error(self):
            self.assert_rejects_drift(lambda s: check_message_attribute(s, "wasm"), wasm_messages,
                                      "OpenUiError::InvalidAttribute", "OpenUiError::InvalidJson")

        def test_swift_case_requires_whole_name(self):
            self.assert_rejects_drift(lambda s: check_error_case(s, "case", "InvalidAttribute"),
                                      "    case InvalidAttribute(reason: String)",
                                      "InvalidAttribute", "InvalidAttributeX")

        def test_kotlin_class_requires_whole_name(self):
            self.assert_rejects_drift(lambda s: check_error_case(s, "class", "InvalidAttribute"),
                                      "    class InvalidAttribute(",
                                      "InvalidAttribute", "InvalidAttributeX")

    unittest.main(argv=[sys.argv[0], *sys.argv[2:]])
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
    assert re.search(r"^export function extractMcpToolResult\(response_json: string, limits: ParseLimits\): string;", declarations, re.M), "wasm MCP helper signature drift"
    assert "extractMcpToolResult" not in session[1], "wasm MCP helper must not be a session static"
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
    library = re.search(r"export class ComponentLibrary\s*\{(.*?)\n\}", declarations, re.S)
    assert library, "missing wasm ComponentLibrary"
    for declaration in ["static fromSchemaWithLimits(schema: string, limits: RuntimeLimits): ComponentLibrary;",
                        "static fromDefinition(definition: string): ComponentLibrary;",
                        "static fromDefinitionWithLimits(definition: string, limits: RuntimeLimits): ComponentLibrary;",
                        "jsonSchema(): string | undefined;", "promptSpec(): string | undefined;", "constructor(schema: string);",
                        "prompt(options?: string | null): string | undefined;",
                        "cloudConfig(options?: string | null, instructions?: string | null): string | undefined;"]:
        assert declaration in library[1], declaration
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
# The generated Kotlin, as UniFFI writes it or as scripts/kotlin-binding.py
# hides it in OpenUICompose (every top-level declaration internal).
def check_kotlin(kotlin):
    for function in reader_functions:
        assert re.search(r"fun `" + function + r"`\([^\n]*\): kotlin.String", kotlin), function
    api = re.search(r"(?:public|internal) interface RuntimeSessionInterface\s*\{(.*?)\n\}", kotlin, re.S)
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
    for error in error_cases:
        check_error_case(kotlin, "class", error)
    library = re.search(r"(?:public|internal) interface ComponentLibraryInterface\s*\{(.*?)\n\}", kotlin, re.S)
    assert library, "missing Kotlin ComponentLibrary interface"
    for method in ["fun `componentNames`(): List<kotlin.String>", "fun `jsonSchema`(): kotlin.String?", "fun `promptSpec`(): kotlin.String?",
                   "fun `prompt`(`options`: kotlin.String?): kotlin.String?",
                   "fun `cloudConfig`(`options`: kotlin.String?, `instructions`: kotlin.String?): kotlin.String?"]:
        assert method in library[1], method
    assert "@Throws(OpenUiException::class) fun `fromSchemaWithLimits`(`schema`: kotlin.String, `limits`: RuntimeLimits): ComponentLibrary" in kotlin, "Kotlin fromSchemaWithLimits drift"
    assert "@Throws(OpenUiException::class) fun `fromDefinition`(`definition`: kotlin.String): ComponentLibrary" in kotlin, "Kotlin fromDefinition drift"
    assert "@Throws(OpenUiException::class) fun `fromDefinitionWithLimits`(`definition`: kotlin.String, `limits`: RuntimeLimits): ComponentLibrary" in kotlin, "Kotlin fromDefinitionWithLimits drift"
if len(sys.argv) == 3 and sys.argv[1] == "--kotlin":
    check_kotlin(pathlib.Path(sys.argv[2]).read_text())
    print(f"Kotlin: {len(native_methods)} session methods, opaque objects, typed errors and current limits checked")
    sys.exit(0)
if len(sys.argv) not in (2, 3):
    sys.exit("usage: check-runtime-binding-api.py [--self-test | --wasm TS_FILE | --kotlin KOTLIN_FILE | SWIFT_FILE [KOTLIN_FILE]]")
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
for error in error_cases:
    check_error_case(swift, "case", error)
library = re.search(r"public protocol ComponentLibraryProtocol[^\{]*\{(.*?)\n\}", swift, re.S)
assert library, "missing Swift ComponentLibrary protocol"
for method in [r"componentNames\(\)\s*->\s*\[String\]", r"jsonSchema\(\)\s*->\s*String\?", r"promptSpec\(\)\s*->\s*String\?",
               r"prompt\(options: String\?\)\s*throws\s*->\s*String\?",
               r"cloudConfig\(options: String\?, instructions: String\?\)\s*throws\s*->\s*String\?"]:
    assert re.search(r"func\s+" + method, library[1]), method
assert re.search(r"public static func fromSchemaWithLimits\(schema: String, limits: RuntimeLimits\)\s*throws\s*-> ComponentLibrary", swift), "Swift fromSchemaWithLimits drift"
assert re.search(r"public static func fromDefinition\(definition: String\)\s*throws\s*-> ComponentLibrary", swift), "Swift fromDefinition drift"
assert re.search(r"public static func fromDefinitionWithLimits\(definition: String, limits: RuntimeLimits\)\s*throws\s*-> ComponentLibrary", swift), "Swift fromDefinitionWithLimits drift"
if len(sys.argv) == 3:
    check_kotlin(pathlib.Path(sys.argv[2]).read_text())
print(f"{'Swift/Kotlin' if len(sys.argv) == 3 else 'Swift'}: {len(native_methods)} session methods, opaque objects, typed errors and current limits checked")
