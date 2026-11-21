// The typed prompt calls (#148) against upstream's own output: every
// recorded prompt case whose options the typed options can say exactly is
// asked for through `systemPrompt(library:options:)` or
// `cloudConfigBlock(library:options:instructions:)` and must give the
// recorded prompt, or the recorded refusal, byte for byte. The recordings
// are conformance/golden/prompts, written by scripts/regen-golden.sh from
// upstream's generateSystemPrompt.
import Foundation
import OpenUINative
import Testing

private let repository: URL = {
    var directory = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { directory.deleteLastPathComponent() }
    return directory
}()

private func recorded(_ folder: String, _ name: String) throws -> JSONValue {
    let url = repository.appendingPathComponent("conformance/\(folder)/\(name).json")
    return try JSONValue.parse(String(contentsOf: url, encoding: .utf8))
}

private func caseNames() throws -> [String] {
    try FileManager.default.contentsOfDirectory(atPath: repository.appendingPathComponent("conformance/prompts").path)
        .filter { $0.hasSuffix(".json") }
        .map { String($0.dropLast(".json".count)) }
        .sorted()
}

/// Options in a recorded spec the typed options can't say exactly.
private struct Untyped: Error {}

private func strings(_ value: JSONValue) throws(Untyped) -> [String] {
    guard let items = value.array else { throw Untyped() }
    return try items.map { item throws(Untyped) in
        guard let text = item.string else { throw Untyped() }
        return text
    }
}

private func bool(_ value: JSONValue) throws(Untyped) -> Bool {
    guard let flag = value.bool else { throw Untyped() }
    return flag
}

private func tool(_ value: JSONValue) throws(Untyped) -> PromptTool {
    if let name = value.string { return .name(name) }
    guard case let .object(entries) = value, let name = value["name"]?.string,
          let input = value["inputSchema"] else { throw Untyped() }
    var spec = ToolSpec(name: name, inputSchema: input)
    for (key, field) in entries {
        switch key {
        case "name", "inputSchema": continue
        case "description":
            guard let text = field.string else { throw Untyped() }
            spec.description = text
        case "outputSchema": spec.outputSchema = field
        case "annotations":
            guard case let .object(hints) = field else { throw Untyped() }
            var annotations = ToolAnnotations()
            for (hint, flag) in hints {
                switch hint {
                case "readOnlyHint": annotations.readOnlyHint = try bool(flag)
                case "destructiveHint": annotations.destructiveHint = try bool(flag)
                default: throw Untyped()
                }
            }
            spec.annotations = annotations
        default: throw Untyped()
        }
    }
    return .spec(spec)
}

/// The typed options that say exactly what a recorded `promptOptions` says.
private func options(_ json: JSONValue?) throws(Untyped) -> PromptOptions {
    guard let json else { return PromptOptions() }
    guard case let .object(entries) = json else { throw Untyped() }
    var options = PromptOptions()
    for (key, value) in entries {
        switch key {
        case "tools":
            guard let items = value.array else { throw Untyped() }
            options.tools = try items.map { item throws(Untyped) in try tool(item) }
        case "editMode": options.editMode = try bool(value)
        case "inlineMode": options.inlineMode = try bool(value)
        case "toolCalls": options.toolCalls = try bool(value)
        case "bindings": options.bindings = try bool(value)
        case "builtinFunctions": options.builtinFunctions = try bool(value)
        case "preamble":
            guard let text = value.string else { throw Untyped() }
            options.preamble = text
        case "examples": options.examples = try strings(value)
        case "toolExamples": options.toolExamples = try strings(value)
        case "additionalRules": options.additionalRules = try strings(value)
        default: throw Untyped()
        }
    }
    return options
}

/// What upstream gave for a recorded case: its prompt, or its refusal.
private enum Recorded: Equatable {
    case prompt(String)
    case refused(String?)

    init(_ golden: JSONValue) {
        if let prompt = golden["prompt"]?.string {
            self = .prompt(prompt)
        } else {
            // An engine crash (`errorType`) only has to be refused too.
            self = .refused(golden["errorType"] == nil ? golden["error"]?.string : nil)
        }
    }

    func matches(_ asked: () async throws(OpenUIError) -> String) async -> Bool {
        do {
            let prompt = try await asked()
            return self == .prompt(prompt)
        } catch {
            guard case let .refused(message) = self else { return false }
            return error.kind == .prompt && (message == nil || message == error.message)
        }
    }
}

@Suite struct PromptTests {
    @Test func theTypedFullPromptIsUpstreamsForEveryRecordedLibraryCase() async throws {
        let core = Core()
        var asked: Set<String> = []
        for name in try caseNames() {
            let spec = try recorded("prompts", name)
            guard case let .object(keys) = spec, keys.allSatisfy({ ["library", "promptOptions"].contains($0.key) }),
                  let library = spec["library"], let typed = try? options(spec["promptOptions"]) else { continue }
            let want = Recorded(try recorded("golden/prompts", name))
            let matched = await want.matches { () async throws(OpenUIError) -> String in
                try await core.systemPrompt(library: library, options: typed)
            }
            #expect(matched, "\(name): the typed full prompt differs from upstream's")
            asked.insert(name)
        }
        for name in ["react-ui-schema", "react-ui-schema-tools", "library-tools-specs", "library-everything",
                     "library-edit-and-inline", "library-defaults", "spec-1.0-library-tools"] {
            #expect(asked.contains(name), "\(name) wasn't asked for")
        }
        #expect(asked.count >= 30, "only \(asked.count) recorded cases were asked for")
    }

    @Test func theTypedCloudConfigBlockIsUpstreamsForEveryRecordedCloudCase() async throws {
        let core = Core()
        var asked: Set<String> = []
        for name in try caseNames() {
            let spec = try recorded("prompts", name)
            guard spec["cloud"] == .bool(true), case let .object(keys) = spec,
                  keys.allSatisfy({ ["cloud", "library", "promptOptions", "instructions"].contains($0.key) }),
                  let typed = try? options(spec["promptOptions"]) else { continue }
            let instructions = spec["instructions"]
            guard instructions == nil || instructions?.string != nil else { continue }
            let want = Recorded(try recorded("golden/prompts", name))
            // Upstream picks the options Cloud carries from whatever it is
            // given; so does `CloudPromptOptions(_:)`.
            let matched = await want.matches { () async throws(OpenUIError) -> String in
                try await core.cloudConfigBlock(library: spec["library"], options: CloudPromptOptions(typed),
                                                 instructions: instructions?.string)
            }
            #expect(matched, "\(name): the typed cloud config block differs from upstream's")
            asked.insert(name)
        }
        for name in ["react-ui-cloud", "cloud-all-options", "cloud-strips-flags", "cloud-library-options",
                     "cloud-no-library", "cloud-instructions", "cloud-options-without-library", "cloud-invalid-library"] {
            #expect(asked.contains(name), "\(name) wasn't asked for")
        }
    }

    /// The openui-cloud template's case, spelled out: react-ui's library and
    /// prompt options, in both modes.
    @Test func reactUIsLibraryAndOptionsGiveUpstreamsPromptInBothModes() async throws {
        let core = Core()
        let spec = try recorded("prompts", "react-ui-schema")
        let library = try #require(spec["library"])
        let reactUI = PromptOptions(
            examples: try strings(#require(spec["promptOptions"]?["examples"])),
            additionalRules: try strings(#require(spec["promptOptions"]?["additionalRules"])))
        let wantFull = try #require(recorded("golden/prompts", "react-ui-schema")["prompt"]?.string)
        let wantCloud = try #require(recorded("golden/prompts", "react-ui-cloud")["prompt"]?.string)
        let full = try await core.systemPrompt(library: library, options: reactUI)
        #expect(full == wantFull)
        let cloud = try await core.cloudConfigBlock(library: library, options: CloudPromptOptions(reactUI))
        #expect(cloud == wantCloud)
        #expect(cloud.hasPrefix("]]>openui:config\n"))
        // The raw spec call gives the same text for the recorded spec.
        let rawSpec = try recorded("prompts", "react-ui-cloud").compact
        #expect(try await core.systemPrompt(spec: rawSpec) == wantCloud)
    }

    @Test func cloudModeCarriesOnlyExamplesPreambleAndAdditionalRules() async throws {
        let core = Core()
        let library = try #require(recorded("prompts", "react-ui-schema")["library"])
        let everything = PromptOptions(
            tools: [.name("search"), .spec(ToolSpec(name: "save", inputSchema: .object([])))],
            editMode: true, inlineMode: true, toolCalls: true, bindings: true, builtinFunctions: true,
            preamble: "Acme.", examples: ["root = Stack([])"], toolExamples: ["q = Query(\"search\", {}, [])"],
            additionalRules: ["No red."])
        let picked = CloudPromptOptions(everything)
        #expect(picked == CloudPromptOptions(examples: ["root = Stack([])"], preamble: "Acme.", additionalRules: ["No red."]))
        let block = try await core.cloudConfigBlock(library: library, options: picked)
        let config = try JSONValue.parse(String(block.dropFirst("]]>openui:config\n".count)))
        guard case let .object(top) = config, case let .object(carried)? = config["systemPromptOptions"],
              case let .object(chatLibrary)? = config["chatLibrary"] else {
            Issue.record("not a config block: \(block.prefix(200))"); return
        }
        // The block holds the library, without its components, and the
        // three options; no tools, tool examples, modes or flags.
        #expect(top.map(\.key) == ["chatLibrary", "systemPromptOptions"])
        #expect(JSONValue.object(carried) == .object([
            ("examples", .array([.string("root = Stack([])")])),
            ("preamble", .string("Acme.")),
            ("additionalRules", .array([.string("No red.")])),
        ]))
        #expect(!chatLibrary.contains { $0.key == "components" })
        #expect(!block.contains("toolExamples") && !block.contains("q = Query"))
        // The same block as asking with the three options alone.
        let alone = try await core.cloudConfigBlock(library: library, options: CloudPromptOptions(
            examples: ["root = Stack([])"], preamble: "Acme.", additionalRules: ["No red."]))
        #expect(alone == block)
    }

    @Test func promptRefusalsArriveTyped() async {
        let core = Core()
        let notJSON = await #expect(throws: OpenUIError.self) { try await core.systemPrompt(spec: "{") }
        #expect(notJSON?.kind == .invalidPromptSpec)
        let missing = await #expect(throws: OpenUIError.self) { try await core.systemPrompt(spec: "{}") }
        #expect(missing?.kind == .prompt && missing?.message == "[generatePrompt] components is missing.")
        let builtIn = await #expect(throws: OpenUIError.self) {
            try await core.cloudConfigBlock(library: nil, options: CloudPromptOptions(preamble: "Acme."))
        }
        #expect(builtIn?.kind == .prompt)
        #expect(builtIn?.message == "[generateSystemPrompt] promptOptions requires a library — the built-in library ignores it.")
    }
}
