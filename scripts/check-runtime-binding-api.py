#!/usr/bin/env python3
"""Check generated domain signatures; also called by swift-binding.sh."""
import pathlib
import re
import sys

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
