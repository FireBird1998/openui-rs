// Each refusal the binding can produce reaches Swift as an `OpenUIError`
// with its own kind (#148). The binding's kinds are provoked for real,
// through the generated binding where no public call reaches them, and
// through OpenUINative's public calls where one does; OpenUINative's own
// kinds come from its public calls.
import Foundation
import OpenUIFFI
@testable import OpenUINative
import Synchronization
import Testing

/// The kind a refusal reaches Swift with; nil when nothing was refused.
private func kind(_ body: () throws -> Void) -> OpenUIError.Kind? {
    do {
        try body()
        return nil
    } catch {
        return OpenUIError(error).kind
    }
}

private let actionSchema = #"{"$defs":{"Box":{"properties":{"action":{}}}}}"#
private let mutationProgram = "$n=0\nm=Mutation(\"m\",{})\nroot=Box(Action([@Run(m),@Set($n,7)]))"

/// A session drawing `program`, and the action at `root.props.action`.
private func drawn(_ program: String = mutationProgram, schema: String = actionSchema,
                   limits: RuntimeLimits? = nil) throws -> (RuntimeSession, EvaluationOutput, ActionHandle) {
    let library = try ComponentLibrary(schema: schema)
    let session = try limits.map { try RuntimeSession.withLimits(library: library, promptSpec: nil, limits: $0) }
        ?? RuntimeSession(library: library, promptSpec: nil)
    try session.setProgram(program: parse(program: program, library: library, rootName: nil).program)
    _ = try session.initialize(persisted: nil)
    let tree = try session.evaluate()
    return (session, tree, try session.actionAt(evaluation: tree.evaluation, path: ["props", "action"]))
}

/// A session with no program.
private func bare(limits: RuntimeLimits? = nil) throws -> RuntimeSession {
    let library = try ComponentLibrary(schema: actionSchema)
    return try limits.map { try RuntimeSession.withLimits(library: library, promptSpec: nil, limits: $0) }
        ?? RuntimeSession(library: library, promptSpec: nil)
}

private func limits(evaluation: ParseLimits = ParseLimits(), maxJsonBytes: UInt64 = 8_388_608,
                    maxContinuations: UInt64 = 64) -> RuntimeLimits {
    RuntimeLimits(evaluation: evaluation, referencePreparation: ParseLimits(),
                  maxJsonBytes: maxJsonBytes, maxContinuations: maxContinuations)
}

/// A library function that tries to reenter its session and keeps the kinds
/// it was refused with.
private final class Reentering: LibraryFunction, @unchecked Sendable {
    weak var session: RuntimeSession?
    let refused = Mutex<[OpenUIError.Kind]>([])
    func call(argumentsJson: String) throws -> String? {
        if let session {
            let refusal = kind { _ = try session.set(name: "$n", json: "9") }
            refused.withLock { $0.append(refusal ?? .unexpected) }
        }
        return argumentsJson
    }
}

@Suite struct BindingRefusalKinds {
    @Test func disposed() throws {
        let session = try bare()
        session.dispose()
        #expect(kind { _ = try session.version() } == .disposed)
    }

    @Test func foreignHandle() throws {
        let (a, _, action) = try drawn()
        let (b, _, _) = try drawn()
        defer { a.dispose(); b.dispose() }
        #expect(kind { _ = try b.run(action: action, message: "", form: nil) } == .foreignHandle)
    }

    @Test func staleHandle() throws {
        let (session, _, action) = try drawn()
        defer { session.dispose() }
        _ = try session.evaluate()
        #expect(kind { _ = try session.run(action: action, message: "", form: nil) } == .staleHandle)
    }

    @Test func consumedHandle() throws {
        let (session, _, action) = try drawn()
        defer { session.dispose() }
        let continuation = try #require(try session.run(action: action, message: "", form: nil).continuation)
        _ = try session.resume(continuation: continuation, succeeded: false)
        #expect(kind { _ = try session.resume(continuation: continuation, succeeded: true) } == .consumedHandle)
    }

    @Test func evictedHandle() throws {
        let (session, _, action) = try drawn(limits: limits(maxContinuations: 1))
        defer { session.dispose() }
        let first = try #require(try session.run(action: action, message: "", form: nil).continuation)
        _ = try session.run(action: action, message: "", form: nil)
        #expect(kind { _ = try session.resume(continuation: first, succeeded: true) } == .evictedHandle)
    }

    @Test func invalidHandle() throws {
        let (session, tree, _) = try drawn()
        defer { session.dispose() }
        #expect(kind { _ = try session.actionAt(evaluation: tree.evaluation, path: ["props", "missing"]) } == .invalidHandle)
    }

    @Test func invalidMarker() throws {
        let session = try bare()
        defer { session.dispose() }
        #expect(kind { _ = try session.writeBinding(marker: "null", json: "1") } == .invalidMarker)
    }

    @Test func invalidJSON() throws {
        let session = try bare()
        defer { session.dispose() }
        #expect(kind { _ = try session.set(name: "$n", json: "{") } == .invalidJSON)
    }

    @Test func refusedWrite() throws {
        let session = try bare(limits: limits(evaluation: ParseLimits(resultSize: 64)))
        defer { session.dispose() }
        let long = "\"" + String(repeating: "x", count: 65) + "\""
        #expect(kind { _ = try session.set(name: "$x", json: long) } == .refusedWrite)
    }

    @Test func evaluation() throws {
        let session = try bare()
        defer { session.dispose() }
        let marker = #"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":{"toString":3}}}"#
        #expect(kind { _ = try session.writeBinding(marker: marker, json: "1") } == .evaluation)
    }

    @Test func limitExceeded() throws {
        let session = try bare(limits: limits(maxJsonBytes: 2048))
        defer { session.dispose() }
        let long = "\"" + String(repeating: "x", count: 2047) + "\""
        #expect(kind { _ = try session.set(name: "a", json: long) } == .limitExceeded)
    }

    @Test func worker() throws {
        let library = try ComponentLibrary(schema: #"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}}}}}"#)
        let session = try RuntimeSession(library: library, promptSpec: nil)
        defer { session.dispose() }
        try session.setProgram(program: parse(program: "$n=0\nroot=Box(@Echo(7))", library: library, rootName: nil).program)
        _ = try session.initialize(persisted: nil)
        let function = Reentering()
        function.session = session
        try session.registerFunction(name: "Echo", callback: function)
        _ = try session.evaluate()
        #expect(function.refused.withLock { $0 } == [.worker])
    }

    @Test func invalidSchemaThroughTheCore() async {
        let refused = await #expect(throws: OpenUIError.self) { try await Core().library(schema: "{") }
        #expect(refused?.kind == .invalidSchema)
    }

    @Test func invalidPromptSpecAndPromptThroughTheCore() async {
        let core = Core()
        let notJSON = await #expect(throws: OpenUIError.self) { try await core.systemPrompt(spec: "{") }
        #expect(notJSON?.kind == .invalidPromptSpec)
        let refused = await #expect(throws: OpenUIError.self) { try await core.systemPrompt(spec: "{}") }
        #expect(refused?.kind == .prompt)
    }

    @Test func referenceConflict() throws {
        let session = try bare()
        defer { session.dispose() }
        try session.setReferenceValues(json: #"{"q":1}"#)
        #expect(kind { try session.attachToolProvider() } == .referenceConflict)
    }

    @Test func invalidToolCall() throws {
        let library = try ComponentLibrary(schema: #"{"$defs":{"Box":{"properties":{"value":{}}}}}"#)
        let session = try RuntimeSession(library: library, promptSpec: nil)
        defer { session.dispose() }
        try session.attachToolProvider()
        let program = parse(program: "q=Query(\"read\",{})\nroot=Box(q)", library: library, rootName: nil).program
        _ = try session.replaceProgram(program: program, persisted: nil, projected: nil, nativeView: true)
        _ = try session.evaluate()
        let call = try #require(try session.takeQueryUpdate().calls.first)
        try session.completeToolCall(call: call, outcomeJson: #"{"value":1}"#)
        #expect(kind { try session.completeToolCall(call: call, outcomeJson: #"{"value":2}"#) } == .invalidToolCall)
    }

    /// A paused mutation is reported in the action's output, not thrown; it
    /// keeps its kind there too.
    @Test func queriesPaused() throws {
        let library = try ComponentLibrary(schema: actionSchema)
        let session = try RuntimeSession(library: library, promptSpec: nil)
        defer { session.dispose() }
        try session.attachToolProvider()
        let program = parse(program: "$n=0\nm=Mutation(\"save\",{})\nroot=Box(Action([@Set($n,1),@Run(m),@Set($n,2)]))",
                            library: library, rootName: nil).program
        let draw = try session.replaceProgram(program: program, persisted: nil, projected: nil, nativeView: true).evaluation
        let action = try session.actionAt(evaluation: draw.evaluation, path: ["props", "action"])
        try session.pauseQueries()
        let failure = try #require(try session.run(action: action, message: "", form: nil).failure)
        #expect(OpenUIError(binding: failure.error).kind == .queriesPaused)
    }

    @Test func invalidAttribute() {
        #expect(kind { _ = try buildMessage(parts: #"{"content":"body","attributes":{"x":"\ud800"}}"#) } == .invalidAttribute)
    }

    /// Every binding kind maps to its own OpenUINative kind, with the
    /// binding's wording.
    @Test func eachBindingKindHasItsOwnKind() {
        let all: [OpenUiError] = [
            .Disposed, .ForeignHandle, .StaleHandle, .ConsumedHandle, .EvictedHandle, .InvalidHandle,
            .InvalidMarker, .InvalidJson(reason: "r"), .RefusedWrite(reason: "r"), .Evaluation(reason: "r"),
            .LimitExceeded(reason: "r"), .Worker(reason: "r"), .InvalidSchema(reason: "r"),
            .InvalidPromptSpec(reason: "r"), .Prompt(reason: "r"), .ReferenceConflict, .InvalidToolCall,
            .QueriesPaused(reason: "r"), .InvalidAttribute(reason: "r"),
        ]
        let kinds = all.map { OpenUIError(binding: $0).kind }
        #expect(Set(kinds).count == all.count)
        #expect(OpenUIError(binding: .InvalidSchema(reason: "r")).message == "invalid JSON Schema: r")
        #expect(OpenUIError(binding: .Prompt(reason: "r")).message == "r")
        #expect(OpenUIError(binding: .Disposed).description == "session disposed")
    }
}

@Suite(.timeLimit(.minutes(1))) @MainActor struct OpenUINativeRefusalKinds {
    private let schema = #"{"$defs":{"Text":{"properties":{"text":{}}},"Button":{"properties":{"label":{},"action":{}}}}}"#

    private func live(_ program: String) async throws -> (Core, Library, LiveProgram) {
        let core = Core()
        let library = try await core.library(schema: schema)
        let live = try LiveProgram(library: library)
        try live.show(try await core.parseOneShot(program, library: library))
        return (core, library, live)
    }

    @Test func disposedThroughALiveProgram() async throws {
        let (_, _, live) = try await live(#"root=Button("Go", @ToAssistant("go"))"#)
        live.session.dispose()
        #expect(kind { try live.set("$n", to: .number(1)) } == .disposed)
    }

    @Test func staleStreamingAndBlocked() async throws {
        let (core, library, live) = try await live(#"root=Button("Go", @ToAssistant("go"))"#)
        let old = try #require(live.root)
        try live.show(try await core.parseOneShot(#"root=Button("Again", @ToAssistant("again"))"#, library: library))
        #expect(kind { try live.run("action", of: old, message: "Go") } == .stale)
        live.isStreaming = true
        #expect(kind { try live.run("action", of: #require(live.root), message: "Go") } == .streaming)
        live.isStreaming = false
        // A settled response bundle with an invalid scripts section blocks
        // interaction; the refusal carries the bundle's error.
        let bundle = "]]>openui:content?name=Desk\n" + #"root=Button("Send", @ToAssistant("sent"))"#
            + "\n]]>openui:scripts {\"count\":1}\n["
        try await live.show(response: bundle, streaming: false)
        let refused = #expect(throws: OpenUIError.self) { try live.enter(.string("x"), into: #require(live.root)) }
        #expect(refused?.kind == .blocked)
        #expect(refused?.message == live.response?.error)
    }

    @Test func unusableElementToolProviderAttachedAndInvalidArgument() async throws {
        let (_, _, live) = try await live(#"root=Text("no name")"#)
        #expect(kind { try live.enter(.string("x"), into: #require(live.root)) } == .unusableElement)
        #expect(kind { try live.attachToolProvider(clock: ManualClock(), minimumRefreshInterval: .microseconds(1500)) { _, _ in .null } } == .invalidArgument)
        try live.attachToolProvider(clock: ManualClock()) { _, _ in .null }
        #expect(kind { try live.attachToolProvider(clock: ManualClock()) { _, _ in .null } } == .toolProviderAttached)
    }

    @Test func decode() {
        #expect(kind { _ = try ParseResult(json: "[", depthLimited: false, sizeLimited: false) } == .decode)
        #expect(kind { _ = try ParseResult(json: "{}", depthLimited: false, sizeLimited: false) } == .decode)
    }

    @Test func limitExceededThroughALiveProgram() async throws {
        let (_, _, live) = try await live(#"root=Text("City")"#)
        let oversized = "]]>openui:content?name=Desk\n" + #"root=Text("City")"#
            + "\n]]>openui:scripts {\"count\":1}\n" + String(repeating: " ", count: 1 << 20)
        let refused = await #expect(throws: OpenUIError.self) { try await live.show(response: oversized, streaming: false) }
        #expect(refused?.kind == .limitExceeded)
    }
}
