import OpenUIFFI
import Testing
@testable import OpenUINative

@Suite @MainActor struct FunctionDefinitionTests {
    let box = defineComponent(name: "Box", description: "", props: [.init("value", .any)])

    @Test func functionDeclarationAppearsInThePromptSpec() async throws {
        let function = defineFunction(name: "Label", description: "Format a label.", params: [
            .init("value", .number), .init("prefix", .string, optional: true)
        ], returns: .string)
        let library = try await Core().createLibrary(components: [box], functions: [function])
        let spec = try JSONValue.parse(#require(try await library.promptSpec()))
        #expect(spec["functions"]?["Label"]?["signature"] == .string("Label(value: number, prefix?: string) → string"))
        #expect(spec["functions"]?["Label"]?["description"] == .string("Format a label."))
    }

    @Test func positionalArgumentsReachTheSwiftClosureByName() async throws {
        let pair = defineFunction(name: "Pair", description: "", params: [
            .init("z", .string), .init("a", .string), .init("unused", .number, optional: true)
        ]) { args in args }
        let library = try await Core().createLibrary(components: [box], functions: [pair])
        let live = try LiveProgram(library: library)
        try await live.show(response: #"root = Box(@Pair("first", "last"))"#, streaming: false)
        #expect(live.rootJSON?["props"]?["value"] == .object([("z", .string("first")), ("a", .string("last"))]))
        #expect(live.errors.isEmpty)
    }

    @Test func eachSessionRegistersOnceAcrossShowsAndPreviews() async throws {
        let read = defineFunction(name: "Read", description: "", params: []) { _ in .string("library") }
        let library = try await Core().createLibrary(components: [box], functions: [read])
        let source = "root = Box(@Read())"
        let live = try LiveProgram(library: library)
        // Replacing the settled session's callback makes a second registration observable.
        try live.session.registerFunction(name: read.name, callback: Replacement())
        for streaming in [false, true, true, false, true, false] {
            try await live.show(response: source, streaming: streaming)
            #expect(live.rootJSON?["props"]?["value"] == .string(streaming ? "library" : "replacement"))
        }
        let other = try LiveProgram(library: library)
        try await other.show(response: source, streaming: false)
        #expect(other.rootJSON?["props"]?["value"] == .string("library"))
    }

    @Test func invalidArgumentInvalidReturnAndThrownErrorAreReportedAsNull() async throws {
        enum Failure: Error { case rejected }
        let check = defineFunction(name: "Check", description: "", params: [.init("n", .number)], returns: .string) { args in
            if args["n"] == .number(1) { return .bool(false) }
            throw Failure.rejected
        }
        let library = try await Core().createLibrary(components: [box], functions: [check])
        let live = try LiveProgram(library: library)
        for (value, reason) in [(#""bad""#, #"@Check: field "/n" expects number but got string"#),
                                ("1", #"@Check: field "/returns" expects string but got boolean"#),
                                ("2", "@Check threw: rejected")] {
            try await live.show(response: "$n = \(value)\nroot = Box(@Check($n))", streaming: false)
            #expect(live.rootJSON?["props"]?["value"] == .null)
            #expect(live.errors.map(\.message) == [#"Evaluating prop "value" on Box failed: "# + reason])
        }
    }

    @Test func declarationWithoutAnImplementationEvaluatesToNull() async throws {
        let pending = defineFunction(name: "Pending", description: "", params: [])
        let library = try await Core().createLibrary(components: [box], functions: [pending])
        let live = try LiveProgram(library: library)
        try await live.show(response: "root = Box(@Pending())", streaming: false)
        #expect(live.rootJSON?["props"]?["value"] == .null)
        #expect(live.errors.isEmpty)
    }

    @Test func sessionLimitCountsOnlyImplementedFunctions() async throws {
        let functions = (1...256).map { n in
            defineFunction(name: "F\(n)", description: "", params: []) { _ in .number(Double(n)) }
        }
        let core = Core()
        let pending = defineFunction(name: "Pending", description: "", params: [])
        let library = try await core.createLibrary(components: [box], functions: functions + [pending])
        let live = try LiveProgram(library: library)
        try await live.show(response: "root = Box(@F256())", streaming: false)
        #expect(live.rootJSON?["props"]?["value"] == .number(256))

        let extra = defineFunction(name: "Extra", description: "", params: []) { _ in .null }
        let over = try await core.createLibrary(components: [box], functions: functions + [extra])
        #expect(throws: OpenUIError(.limitExceeded, "function registration limit")) {
            try LiveProgram(library: over)
        }
    }

    @Test func declaredActionSendsItsCheckedParamsToTheApp() async throws {
        let copy = defineAction(name: "Copy", description: "Copy text.", params: [
            .init("text", .string), .init("count", .number, optional: true)
        ])
        let control = defineComponent(name: "Control", description: "", props: [.init("onTap", .action(copy.name))])
        let library = try await Core().createLibrary(components: [control], actions: [copy])
        let spec = try JSONValue.parse(#require(try await library.promptSpec()))
        #expect(spec["actions"]?["Copy"]?["signature"] == .string("Copy(text: string, count?: number)"))
        #expect(spec["actions"]?["Copy"]?["description"] == .string("Copy text."))

        var events: [HostEvent] = []
        let live = try LiveProgram(library: library) { events.append($0) }
        try await live.show(response: #"root = Control(@Copy("Hello", 2))"#, streaming: false)
        try live.run("onTap", of: #require(live.root), message: "Tap")
        #expect(events.map(\.type) == ["Copy"])
        #expect(events.first?.params == .object([("text", .string("Hello")), ("count", .number(2))]))
    }

    @Test func collidingDeclarationsGiveTheCoresReason() async throws {
        let core = Core()
        func refused(_ name: String, functions: [String], actions: [String], kind: String) async {
            let reason = "[createLibrary] \(kind) \"\(name)\" collides with a built-in, component, function, or action of the same name. Rename it."
            await #expect(throws: OpenUIError(.invalidJSON, reason)) {
                try await core.createLibrary(components: [box],
                    functions: functions.map { defineFunction(name: $0, description: "", params: []) },
                    actions: actions.map { defineAction(name: $0, description: "", params: []) })
            }
        }
        for name in ["Sum", "Query", "Box"] {
            await refused(name, functions: [name], actions: [], kind: "Function")
            await refused(name, functions: [], actions: [name], kind: "Action")
        }
        await refused("Clash", functions: ["Clash", "Clash"], actions: [], kind: "Function")
        await refused("Clash", functions: ["Clash"], actions: ["Clash"], kind: "Action")
        await refused("Clash", functions: [], actions: ["Clash", "Clash"], kind: "Action")
    }

    @Test func callbackResultEncodingStopsAtItsByteLimit() {
        let value = JSONValue.object([("é", .array([.string("\n"), .number(12)]))])
        let bytes = value.compact.utf8.count
        #expect(value.compact(byteLimit: bytes) == value.compact)
        #expect(value.compact(byteLimit: bytes - 1) == nil)

        var shared = JSONValue.null
        for _ in 0..<40 { shared = .array([shared, shared]) }
        #expect(shared.compact(byteLimit: 128) == nil)
    }
}

private final class Replacement: LibraryFunction {
    func call(argumentsJson: String) throws -> String? { #""replacement""# }
}
