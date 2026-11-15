#!/usr/bin/env python3
"""Check generated domain signatures; also called by swift-binding.sh."""
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent
methods = [
    "actionAt", "dispose", "evaluate", "get", "getField", "initialize",
    "initializeField", "isPristine", "markBinding", "pendingContinuations",
    "resume", "run", "runLegacy", "set", "setField", "setProgram",
    "setReferenceValues", "snapshot", "undecidedBindings", "version", "writeBinding",
]
def camel(name):
    return re.sub(r"_([a-z])", lambda m: m[1].upper(), name)
native = (root / "crates/openui-ffi/src/runtime.rs").read_text()
wasm = (root / "crates/openui-wasm/src/runtime.rs").read_text()
inner = (root / "crates/openui-wasm/src/session.rs").read_text()
core = (root / "crates/openui-session/src/lib.rs").read_text()
for source, label in [(native, "native"), (inner, "wasm inner"), (core, "session core")]:
    exported = {camel(n) for n in re.findall(r"pub fn (\w+)\(", source)}
    assert set(methods) - {"dispose", "setReferenceValues"} <= exported, label
native_session = native.split("#[uniffi::export]", 1)[1].split("fn worker_loop", 1)[0]
assert {camel(n) for n in re.findall(r"pub fn (\w+)\(", native_session)} == set(methods) | {"new", "withLimits"}, "native session drift"
ffi_lib = (root / "crates/openui-ffi/src/lib.rs").read_text()
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
    native_fields = set(re.findall(r"pub (\w+):", source.split("pub struct " + record_name + " {", 1)[1].split("\n}", 1)[0]))
    core_fields = set(re.findall(r"pub (\w+):", core.split("pub struct " + record_name + " {", 1)[1].split("\n}", 1)[0]))
    assert native_fields == core_fields, record_name + " core/native drift"
    getters, setters = wasm_accessors("ParseLimits" if record_name == "ParseLimits" else "RuntimeLimits")
    expected = {camel(field) for field in native_fields}
    assert getters == expected, f"{record_name} wasm getters {sorted(getters ^ expected)} drift"
    assert setters == expected, f"{record_name} wasm setters {sorted(setters ^ expected)} drift"
wasm_session = wasm.split("impl RuntimeSession {", 1)[1].split("pub struct RuntimeLimits", 1)[0]
assert set(re.findall(r"js_name = (\w+)", wasm_session)) == set(methods) | {"withLimits"}, "wasm session drift"
assert "action: Option<ActionHandle>" in wasm_session
assert "evaluation: &EvaluationHandle" in wasm_session
assert "continuation: &ContinuationHandle" in wasm_session
assert "program: &ParsedProgram" in wasm_session
for name in ["ForeignHandle", "StaleHandle", "ConsumedHandle", "EvictedHandle", "InvalidHandle", "InvalidJson", "InvalidMarker", "RefusedWrite", "Disposed", "Evaluation", "LimitExceeded", "Worker"]:
    assert name in core and name in wasm and name in (root / "crates/openui-ffi/src/lib.rs").read_text(), name
assert "package-guards.js" in (root / "scripts/wasm.sh").read_text()
print(f"Source: {len(methods)} native/wasm session methods, opaque capabilities, errors and limits checked")
if len(sys.argv) == 1:
    sys.exit(0)
if len(sys.argv) == 3 and sys.argv[1] == "--wasm":
    declarations = pathlib.Path(sys.argv[2]).read_text()
    session = re.search(r"export class RuntimeSession\s*\{(.*?)\n\}", declarations, re.S)
    assert session, "missing wasm RuntimeSession"
    for method in methods + ["withLimits"]:
        assert re.search(r"\b" + method + r"\(", session[1]), method
    for name in ["ParsedProgram", "EvaluationHandle", "ActionHandle", "ContinuationHandle", "RuntimeLimits", "ParseLimits", "EvaluationOutput", "ActionOutput"]:
        assert "export class " + name in declarations, name
    # Each limit field is both readable and writable under its native name.
    for record_name, fields in [("RuntimeLimits", ["evaluation", "referencePreparation", "maxJsonBytes", "maxContinuations"]),
                                ("ParseLimits", ["programSize", "resultSize", "resultFloor", "amplification", "resultCeiling", "depth", "work"])]:
        body = re.search(r"export class " + record_name + r"\s*\{(.*?)\n\}", declarations, re.S)[1]
        for field in fields:
            plain = re.search(r"^\s*" + field + r":", body, re.M)
            assert plain or (re.search(r"^\s*get " + field + r"\(", body, re.M) and re.search(r"^\s*set " + field + r"\(", body, re.M)), record_name + "." + field
    print("wasm-bindgen declarations checked")
    sys.exit(0)
if len(sys.argv) not in (2, 3):
    sys.exit("usage: check-runtime-binding-api.py SWIFT_FILE [KOTLIN_FILE]")
swift = pathlib.Path(sys.argv[1]).read_text()
api = re.search(r"public protocol RuntimeSessionProtocol[^\{]*\{(.*?)\n\}", swift, re.S)
assert api, "missing Swift RuntimeSession protocol"
methods = [
    "actionAt", "dispose", "evaluate", "get", "getField", "initialize",
    "initializeField", "isPristine", "markBinding", "pendingContinuations",
    "resume", "run", "runLegacy", "set", "setField", "setProgram",
    "setReferenceValues", "snapshot", "undecidedBindings", "version", "writeBinding",
]
for method in methods:
    assert re.search(r"func\s+" + method + r"\(", api[1]), method
    if method != "dispose":
        assert re.search(r"func\s+" + method + r"\([^\n]*\)\s*throws", api[1]), method
for name, kind in [("action", "ActionHandle?"), ("continuation", "ContinuationHandle"),
                   ("evaluation", "EvaluationHandle"), ("program", "ParsedProgram")]:
    assert f"{name}: {kind}" in api[1]
assert "public var referencePreparation: ParseLimits" in swift
assert "public var maxPlans:" not in swift
assert "compile_fail" not in swift, "rustdoc-only snippet leaked to Swift"
for error in ["ForeignHandle", "ConsumedHandle", "EvictedHandle", "Evaluation"]:
    assert f"case {error}" in swift
if len(sys.argv) == 3:
    kotlin = pathlib.Path(sys.argv[2]).read_text()
    api = re.search(r"public interface RuntimeSessionInterface\s*\{(.*?)\n\}", kotlin, re.S)
    assert api, "missing Kotlin RuntimeSession interface"
    for method in methods:
        assert f"`{method}`(" in api[1], method
    for name, kind in [("action", "ActionHandle?"), ("continuation", "ContinuationHandle"),
                       ("evaluation", "EvaluationHandle"), ("program", "ParsedProgram")]:
        assert f"`{name}`: {kind}" in api[1]
    assert "@Throws(OpenUiException::class)" in kotlin
    assert "`referencePreparation`: ParseLimits" in kotlin
    assert "`maxPlans`:" not in kotlin
    assert "onMalformedInput(CodingErrorAction.REPORT)" in kotlin
    assert "encode(CharBuffer.wrap(value))" in kotlin
    assert "compile_fail" not in kotlin
    for error in ["ForeignHandle", "ConsumedHandle", "EvictedHandle", "Evaluation"]:
        assert f"class {error}" in kotlin
print(f"{'Swift/Kotlin' if len(sys.argv) == 3 else 'Swift'}: {len(methods)} session methods, opaque objects, typed errors and current limits checked")
