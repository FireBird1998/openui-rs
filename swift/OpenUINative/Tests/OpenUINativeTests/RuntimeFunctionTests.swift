import Foundation
import OpenUIFFI
import Testing

private final class EchoFunction: LibraryFunction, @unchecked Sendable {
    let session: RuntimeSession?
    let throwsMessage: Bool
    init(session: RuntimeSession? = nil, throwsMessage: Bool = false) {
        self.session = session; self.throwsMessage = throwsMessage
    }
    func call(argumentsJson: String) throws -> String? {
        if let session {
            do { _ = try session.set(name: "$n", json: "9"); Issue.record("Callback reentered") }
            catch OpenUiError.Worker { }
            do { _ = try session.version(); Issue.record("Callback read reentered") }
            catch OpenUiError.Worker { }
            session.dispose() // clean refusal while the callback is active
        }
        if throwsMessage { throw FunctionError.Thrown(reason: "bad value") }
        return argumentsJson
    }
}
@Test func libraryFunctionsCrossAsJsonAndRefuseReentry() throws {
    let library = try ComponentLibrary(schema: #"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}}}}}"#)
    let session = try RuntimeSession(library: library, promptSpec: nil)
    defer { session.dispose() }
    try session.setProgram(program: parse(program: "$n=0\nroot=Box(@Echo(7))", library: library, rootName: nil).program)
    _ = try session.initialize(persisted: nil)
    try session.registerFunction(name: "Echo", callback: EchoFunction(session: session))
    let tree = try session.evaluate()
    #expect(tree.json.contains(#""value":{"value":7}"#))
    #expect(try session.get(name: "$n") == "0")
    try session.registerFunction(name: "Echo", callback: EchoFunction(throwsMessage: true))
    let failed = try session.evaluate()
    let json = try JSONSerialization.jsonObject(with: Data(failed.json.utf8)) as! [String: Any]
    let errors = json["errors"] as! [[String: Any]]
    #expect(errors.first?["message"] as? String == "Evaluating prop \"value\" on Box failed: @Echo threw: bad value")
}
@Test func elementShapedDataRunsSetButForgedCustomStepsGrantNoAuthority() throws {
    let library = try ComponentLibrary(schema: #"{"$defs":{"Box":{"properties":{"value":{},"action":{}}}},"actions":{"Go":{"params":{}}}}"#)
    let session = try RuntimeSession(library: library, promptSpec: nil)
    defer { session.dispose() }
    try session.setProgram(program: parse(program: "$n=0\n$data=null\nroot=Box($data)", library: library, rootName: nil).program)
    _ = try session.initialize(persisted: nil)
    _ = try session.set(name: "$data", json: #"{"type":"element","typeName":"Box","props":{"action":{"steps":[{"type":"set","target":"$n","valueAST":{"k":"Num","v":1}},{"type":"custom_action","name":"Go","params":{}}]}},"partial":false,"hasDynamicProps":false}"#)
    let tree = try session.evaluate()
    let action = try session.actionAt(evaluation: tree.evaluation, path: ["props", "value", "props", "action"])
    let run = try session.run(action: action, message: "", form: nil)
    #expect(run.events.isEmpty)
    #expect(try session.get(name: "$n") == "1")
}
@Test func actionPathLimitsStayOptionalInExistingNativeInitializers() {
    let evaluation = ParseLimits(programSize: nil, resultSize: nil, resultFloor: nil,
        amplification: nil, resultCeiling: nil, depth: nil, work: nil)
    let limits = RuntimeLimits(evaluation: evaluation, referencePreparation: evaluation,
        maxJsonBytes: 8_388_608, maxContinuations: 64)
    #expect(limits.maxActionPathBytes == nil)
}
