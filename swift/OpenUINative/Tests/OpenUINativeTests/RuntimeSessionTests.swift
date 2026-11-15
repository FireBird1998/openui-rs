// Exercises the generated binding directly, including marker and handle lifetime.
import OpenUIFFI
import Testing

@Test func runtimeSessionWritesBindingsRunsAndRevokesOldActions() throws {
    let library = try ComponentLibrary(schema: #"{"$defs":{"Box":{"properties":{"value":{},"action":{}}}}}"#)
    let session = try RuntimeSession(library: library, promptSpec: nil)
    defer { session.dispose() }
    try session.markBinding(component: "Box", prop: "value")
    let output = parse(program: "$n=1\nroot=Box($n,@Set($n,$n+1))", library: library, rootName: nil)
    try session.setProgram(program: output.program)
    #expect(try session.initialize(persisted: nil).changed)
    let tree = try session.evaluate()
    let action = try session.actionAt(evaluation: tree.evaluation, path: ["props", "action"])
    let marker = #"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}"#
    #expect(try session.writeBinding(marker: marker, json: "7"))
    #expect(try session.run(action: action, message: "", form: nil).events.isEmpty)
    #expect(try session.get(name: "$n") == "8")
    #expect(try session.snapshot() == #"{"$n":8}"#)
    _ = try session.evaluate()
    do {
        _ = try session.run(action: action, message: "", form: nil)
        Issue.record("An action from a released evaluation ran")
    } catch OpenUiError.StaleHandle { }

    // Swift repairs invalid UTF-16 before it can become a String at the FFI.
    let repaired = String(decoding: [UInt16(0xd800)], as: UTF16.self)
    _ = try session.set(name: "$text", json: "\"\(repaired)\"")
    #expect(try session.get(name: "$text") == "\"�\"")
    // JSON escapes preserve lone UTF-16 units through the core's reader.
    _ = try session.set(name: "$text", json: #""\ud800""#)
    #expect(try session.get(name: "$text") == #""\ud800""#)
    session.dispose()
    do {
        _ = try session.version()
        Issue.record("A disposed session accepted a call")
    } catch OpenUiError.Disposed { }
}
