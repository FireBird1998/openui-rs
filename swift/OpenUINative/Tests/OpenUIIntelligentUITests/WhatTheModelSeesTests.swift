// The Intelligent UI library as the model sees it, against upstream's own
// output: conformance/real-libraries/intelligent-ui.json, written by
// reference/real-library-spec.ts from upstream's package at the library pin
// in reference/UPSTREAM. Its JSON Schema, prompt spec and prompt must be
// upstream's byte for byte once the listed additions are taken out. That
// includes the 59 functions, which this library declares as upstream does.
import Foundation
import OpenUIFFI
import OpenUIIntelligentUI
@testable import OpenUINative
import Testing

private let repository: URL = {
    var directory = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { directory.deleteLastPathComponent() }
    return directory
}()

private func upstream() throws -> JSONValue {
    let url = repository.appendingPathComponent("conformance/real-libraries/intelligent-ui.json")
    return try JSONValue.parse(String(contentsOf: url, encoding: .utf8))
}

/// This library's text with every addition replaced by upstream's text, and
/// how many times each addition was found.
private func removingAdditions(_ text: String) -> (text: String, found: [Int]) {
    var out = text
    var found: [Int] = []
    for change in IntelligentUIAdditions.changes {
        found.append(out.components(separatedBy: change.added).count - 1)
        out = out.replacingOccurrences(of: change.added, with: change.upstream)
    }
    return (out, found)
}

@Suite struct WhatTheModelSeesTests {
    @Test func theSchemaSpecAndPromptAreUpstreamsOnceTheAdditionsAreTakenOut() async throws {
        let fixture = try upstream()
        let core = Core()
        let library = try await core.intelligentUILibrary()

        // Tie the recorded documents to their bytes: the core writes
        // upstream's exact text for upstream's definition (proved against
        // upstream in the Rust tests), and the recording reads back to it.
        let upstreamSchema = try #require(fixture["schema"])
        let upstreamSpec = try #require(fixture["spec"])
        let upstreamOptions = try #require(fixture["promptOptions"])
        let upstreamPrompt = try #require(fixture["prompt"]?.string)
        let definition = try #require(fixture["definition"]).compact
        let (coreSchema, coreSpec) = try await core.run {
            let full = try ComponentLibrary.fromDefinition(definition: definition)
            return (full.jsonSchema(), full.promptSpec())
        }
        #expect(coreSchema == upstreamSchema.compact)
        #expect(coreSpec == upstreamSpec.compact)
        func prompt(_ spec: JSONValue) async throws -> String {
            try await core.systemPrompt(spec: JSONValue.object([("library", spec), ("promptOptions", upstreamOptions)]).compact)
        }
        #expect(try await prompt(upstreamSpec) == upstreamPrompt)

        // Upstream's side, functions and all.
        #expect(upstreamSpec["functions"]?.isObject == true && upstreamSchema["functions"]?.isObject == true)
        let expectedSchema = upstreamSchema.compact
        let expectedSpec = upstreamSpec.compact
        let expectedPrompt = upstreamPrompt

        // This library's side, with the additions taken out.
        let schema = try #require(try await core.run { library.inner.jsonSchema() })
        let spec = try #require(try await library.promptSpec())
        let ownPrompt = try await core.systemPrompt(library: JSONValue.parse(spec), options: IntelligentUI.promptOptions)
        let libraryPrompt = try await core.run { try library.inner.prompt(options: IntelligentUI.promptOptions.json.compact) }
        #expect(libraryPrompt == ownPrompt)

        let (schemaText, schemaFound) = removingAdditions(schema)
        let (specText, specFound) = removingAdditions(spec)
        let (promptText, promptFound) = removingAdditions(ownPrompt)
        // Each addition is where it belongs: the type's values in the schema
        // (also in the spec's copy of it), the signature in the spec (as JSON
        // text) and the prompt, and the description everywhere it is written.
        #expect(schemaFound == [1, 0, 0, 1])
        #expect(specFound == [1, 1, 0, 2])
        #expect(promptFound == [0, 0, 1, 1])
        #expect(schemaText == expectedSchema)
        #expect(specText == expectedSpec)
        #expect(promptText == expectedPrompt)
        // Without that, they differ: the additions are real.
        #expect(schema != expectedSchema && spec != expectedSpec && ownPrompt != expectedPrompt)
    }

    /// docs/library-definition.md's example, as written there.
    @Test func theDocumentedExampleWritesThePrompt() async throws {
        let core = Core()
        let library = try await core.intelligentUILibrary()
        let spec = try JSONValue.parse(try await library.promptSpec() ?? "{}")
        let prompt = try await core.systemPrompt(library: spec, options: IntelligentUI.promptOptions)
        #expect(prompt.contains(#"Chart(type: "bar" | "line" | "area" | "pie" | "donut" | "scatter" | "radar" | "radial", "#))
        #expect(prompt.contains(IntelligentUI.examples[2]))
    }

    @Test func thePromptOptionsAreUpstreams() throws {
        let options = try #require(try upstream()["promptOptions"])
        guard case let .object(entries) = options else { throw JSONError(offset: 0, reason: "not an object") }
        #expect(entries.map(\.key) == ["bindings", "additionalRules", "examples"])
        #expect(IntelligentUI.promptOptions.bindings == options["bindings"]?.bool)
        #expect(IntelligentUI.promptOptions.additionalRules.map { $0.map(JSONValue.string) } == options["additionalRules"]?.array)
        #expect(IntelligentUI.promptOptions.examples.map { $0.map(JSONValue.string) } == options["examples"]?.array)
        #expect(IntelligentUI.additionalRules.count == 19)
        #expect(IntelligentUI.examples.count == 3)
        // Nothing else is set.
        var rest = IntelligentUI.promptOptions
        rest.bindings = nil
        rest.additionalRules = nil
        rest.examples = nil
        #expect(rest == PromptOptions())
    }

    @Test func fiftyTwoComponentsInSixGroupsUnderColEachWithAView() throws {
        let names = IntelligentUI.components.map(\.name)
        let spec = try #require(try upstream()["spec"])
        guard case let .object(components)? = spec["components"] else { throw JSONError(offset: 0, reason: "no components") }
        #expect(names == components.map(\.key))
        #expect(names.count == 52)
        #expect(IntelligentUI.componentGroups.count == 6)
        #expect(IntelligentUI.root == "Col")
        #expect(IntelligentUI.components.allSatisfy { $0.draw != nil })
        // The additions stay on Chart, the one place they belong.
        #expect(IntelligentUIAdditions.chartTypes == ["radar", "radial"])
    }

    @Test func fiftyNineFunctionsInUpstreamsOrder() throws {
        let spec = try #require(try upstream()["spec"])
        guard case let .object(functions)? = spec["functions"] else { throw JSONError(offset: 0, reason: "no functions") }
        #expect(IntelligentUI.functions.map(\.name) == functions.map(\.key))
        #expect(IntelligentUI.functions.count == 59)
    }
}

private extension JSONValue {
    var isObject: Bool {
        if case .object = self { true } else { false }
    }
}
