import Foundation
import OpenUINative
import Testing
@testable import ChatKit

@MainActor
final class MemoryPreferences: ChatPreferenceStore {
    private var values: [String: String] = [:]

    func string(forKey key: String) -> String? { values[key] }
    func set(_ value: String?, forKey key: String) { values[key] = value }
}

@Suite @MainActor struct ChatPreferencesTests {
    @Test func theSelectedModelIsGlobalAndSurvivesRelaunch() async throws {
        let store = MemoryPreferences()
        let preferences = ChatPreferences(store: store)
        let recording = try Recording.named("streamed-answer")
        let transport = RecordedTransport(recording, recording, recording, recording)
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport)
        let library = try await ChatLibrary.reactUI()
        let first = ChatThread(library: library, gateway: gateway, preferences: preferences)
        let second = ChatThread(library: library, gateway: gateway, preferences: preferences)

        first.send("First")
        await first.waitUntilIdle()
        preferences.selectModel("openai/gpt-5.4-mini")
        first.send("Follow-up")
        await first.waitUntilIdle()
        second.send("Another thread")
        await second.waitUntilIdle()

        let relaunched = ChatThread(library: library, gateway: gateway, preferences: ChatPreferences(store: store))
        relaunched.send("After relaunch")
        await relaunched.waitUntilIdle()

        #expect(try transport.sent.map { try JSONValue.parse($0.bodyText)["model"]?.string } == [
            "anthropic/claude-sonnet-5.5", "openai/gpt-5.4-mini", "openai/gpt-5.4-mini", "openai/gpt-5.4-mini",
        ])
        #expect([first.error, second.error, relaunched.error].allSatisfy { $0 == nil })
    }

    @Test(arguments: [
        ("default-model-refused-http", "The model anthropic/claude-sonnet-5.5 does not exist."),
        ("default-model-refused-sse", "Unknown model: anthropic/claude-sonnet-5.5"),
    ])
    func aRefusedDefaultSelectsTheTemplateDefaultForTheNextSendAndRelaunch(
        recording: String, message: String
    ) async throws {
        let store = MemoryPreferences()
        let preferences = ChatPreferences(store: store)
        let transport = RecordedTransport(try .named(recording), try .named("streamed-answer"))
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport)
        let library = try await ChatLibrary.reactUI()
        let thread = ChatThread(library: library, gateway: gateway, preferences: preferences)
        thread.send("Use the default")
        await thread.waitUntilIdle()

        #expect(thread.error == message)
        #expect(transport.sent.count == 1)
        #expect(preferences.modelID == "google/gemini-3.6-flash-free")

        let relaunched = ChatThread(library: library, gateway: gateway, preferences: ChatPreferences(store: store))
        relaunched.send("Use the fallback")
        await relaunched.waitUntilIdle()
        #expect(try transport.sent.map { try JSONValue.parse($0.bodyText)["model"]?.string } == [
            "anthropic/claude-sonnet-5.5", "google/gemini-3.6-flash-free",
        ])
        #expect(relaunched.error == nil)
    }

    @Test(arguments: [
        ("http-error", "anthropic/claude-sonnet-5.5"),
        ("error-mid-stream", "anthropic/claude-sonnet-5.5"),
        ("default-model-refused-http", "openai/gpt-5.4-mini"),
    ])
    func otherFailuresLeaveTheModelChoiceAlone(recording: String, selected: String) async throws {
        let store = MemoryPreferences()
        let preferences = ChatPreferences(store: store)
        preferences.selectModel(selected)
        let transport = RecordedTransport(try .named(recording))
        let thread = ChatThread(library: try await ChatLibrary.reactUI(),
                                gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport),
                                preferences: preferences)
        thread.send("Hello")
        await thread.waitUntilIdle()
        #expect(thread.error != nil)
        #expect(transport.sent.count == 1)
        #expect(preferences.modelID == selected)
        #expect(ChatPreferences(store: store).modelID == selected)
    }

    @Test(arguments: [
        (404, #"{"error":{"message":"The model anthropic/claude-sonnet-5.5 does not exist."}}"#, true),
        (200, #"{"error":{"message":"The model anthropic/claude-sonnet-5.5 does not exist."}}"#, true),
        (404, #"{"error":{"message":"Unknown model"}}"#, true),
        (200, #"{"error":{"message":"Unknown model: anthropic/claude-sonnet-5.5"}}"#, true),
        (422, #"{"error":{"code":"unsupported_model","message":"The requested model is unavailable."}}"#, true),
        (400, #"{"error":{"code":"invalid_parameter","message":"Unsupported model parameter: temperature"}}"#, false),
        (400, #"{"error":{"code":"invalid_parameter","message":"Unknown model"}}"#, false),
        (400, #"{"error":{"message":"Unsupported model parameter: temperature"}}"#, false),
        (200, #"{"error":{"message":"Unknown model: openai/gpt-5.4-mini"}}"#, false),
        (404, #"{"error":{"code":"model_not_found","message":"The model openai/gpt-5.4-mini does not exist."}}"#, false),
        (404, #"{"error":{"code":"model_not_found","message":"Unknown model:anthropic/claude-sonnet-5.5"}}"#, true),
        (404, #"{"error":{"code":"model_not_found","message":"Unknown model: anthropic/claude-sonnet-5.5:other"}}"#, false),
        (404, #"{"error":{"message":"The model anthropic/claude-sonnet-5.50 does not exist."}}"#, false),
        (429, #"{"error":{"message":"Unknown model"}}"#, false),
        (401, #"{"error":{"code":"model_not_found","message":"Unknown model"}}"#, false),
    ])
    func fallbackRequiresAnExplicitRefusalOfTheDefault(status: Int, body: String, fallsBack: Bool) async throws {
        let store = MemoryPreferences()
        let preferences = ChatPreferences(store: store)
        let chunk = status == 200 ? "data: \(body)\n\n" : body
        let transport = RecordedTransport(Recording(status: status, chunks: [chunk]))
        let thread = ChatThread(library: try await ChatLibrary.reactUI(),
                                gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport),
                                preferences: preferences)
        thread.send("Hello")
        await thread.waitUntilIdle()

        let message = try #require(try JSONValue.parse(body)["error"]?["message"]?.string)
        #expect(thread.error == message)
        #expect(transport.sent.count == 1)
        let expected = fallsBack ? "google/gemini-3.6-flash-free" : "anthropic/claude-sonnet-5.5"
        #expect(preferences.modelID == expected)
        #expect(ChatPreferences(store: store).modelID == expected)
    }

    @Test func theRefusalCalloutRedactsTheBearerAndStillFallsBack() async throws {
        let body = #"{"error":{"code":"model_not_found","message":"The model anthropic/claude-sonnet-5.5 is unavailable for key \#(testKey)."}} "#
        let transport = RecordedTransport(Recording(status: 404, chunks: [body]))
        let preferences = ChatPreferences(store: MemoryPreferences())
        let thread = ChatThread(library: try await ChatLibrary.reactUI(),
                                gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport),
                                preferences: preferences)
        thread.send("Hello")
        await thread.waitUntilIdle()
        #expect(thread.error == "The model anthropic/claude-sonnet-5.5 is unavailable for key [your API key].")
        #expect(preferences.modelID == "google/gemini-3.6-flash-free")
        #expect(transport.sent.count == 1)
    }

    @Test(arguments: [false, true])
    func refusedResponseDoesNotPullOneBytePastItsCap(heldOpen: Bool) async throws {
        let cap = 65_536
        let json = #"{"error":{"code":"model_not_found","message":"Unknown model"}}"#
        let cappedBody = json + String(repeating: " ", count: cap - json.utf8.count)
        let (remaining, release) = AsyncStream<UInt8>.makeStream()
        defer { release.finish() }
        let transport = ProbedTransport(status: 404, body: cappedBody + (heldOpen ? "" : "x"),
                                        remaining: heldOpen ? remaining : nil)
        let preferences = ChatPreferences(store: MemoryPreferences())
        let thread = ChatThread(library: try await ChatLibrary.reactUI(),
                                gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport),
                                preferences: preferences)
        thread.send("Hello")

        let settled = await eventually { !thread.isRunning }
        #expect(settled)
        if !settled { thread.stop() }
        await thread.waitUntilIdle()
        #expect(transport.pulls.count == cap)
        #expect(transport.released.isSet)
        #expect(thread.error == "Unknown model")
        #expect(preferences.modelID == "google/gemini-3.6-flash-free")
    }

    @Test func eachPromptModeSendsTheExactCoreGeneratedSystemMessage() async throws {
        let store = MemoryPreferences()
        let preferences = ChatPreferences(store: store)
        let recording = try Recording.named("streamed-answer")
        let transport = RecordedTransport(recording, recording, recording, recording)
        let library = try await ChatLibrary.reactUI()
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport)
        let thread = ChatThread(library: library, gateway: gateway, preferences: preferences)
        #expect(preferences.promptMode == .cloud)
        thread.send("Use cloud")
        await thread.waitUntilIdle()
        preferences.promptMode = .full
        thread.send("Use full")
        await thread.waitUntilIdle()

        let relaunched = ChatThread(library: library, gateway: gateway, preferences: ChatPreferences(store: store))
        relaunched.send("Use the launch default")
        await relaunched.waitUntilIdle()
        #expect(preferences.promptMode == .full)

        preferences.promptMode = .cloud
        thread.send("Back to cloud")
        await thread.waitUntilIdle()

        let cloud = try promptGolden("react-ui-cloud")
        let full = try promptGolden("react-ui-schema")
        let sent = try transport.sent.map {
            let system = try #require(try JSONValue.parse($0.bodyText)["messages"]?.array?.first)
            #expect(system["role"]?.string == "system")
            return Array(try #require(system["content"]?.string).utf8)
        }
        #expect(sent.elementsEqual([cloud, full, cloud, cloud], by: ==))
    }

    private func promptGolden(_ name: String) throws -> [UInt8] {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let source = try String(contentsOf: root.appendingPathComponent("conformance/golden/prompts/\(name).json"), encoding: .utf8)
        return Array(try #require(try JSONValue.parse(source)["prompt"]?.string).utf8)
    }

    @Test func theModelMenuUsesTheTemplateGroupsAndLabelsWithSonnet55Added() {
        let groups = ChatModel.groups.map { group in
            ([group.name] + group.models.map { "\($0.id) | \($0.name)" }).joined(separator: "\n")
        }
        #expect(groups.joined(separator: "\n\n") == """
            Included
            google/gemini-3.1-pro-free | Gemini 3.1 Pro
            google/gemini-3.1-flash-lite-free | Gemini 3.1 Flash Lite
            google/gemini-3.6-flash-free | Gemini 3.6 Flash
            google/gemini-3.5-flash-free | Gemini 3.5 Flash

            Anthropic
            anthropic/claude-sonnet-5.5 | Claude Sonnet 5.5
            anthropic/claude-sonnet-5 | Claude Sonnet 5
            anthropic/claude-sonnet-4.6 | Claude Sonnet 4.6
            anthropic/claude-opus-4-7 | Claude Opus 4.7

            OpenAI
            openai/gpt-5.5 | GPT-5.5
            openai/gpt-5.4 | GPT-5.4
            openai/gpt-5.4-mini | GPT-5.4 mini
            openai/gpt-5.2 | GPT-5.2
            openai/gpt-5.1 | GPT-5.1
            openai/gpt-5 | GPT-5

            Google
            google/gemini-3.6-flash | Gemini 3.6 Flash
            google/gemini-3.5-flash | Gemini 3.5 Flash
            google/gemini-3.1-pro-preview | Gemini 3.1 Pro Preview
            """)
    }

    @Test func aLateDefaultRefusalKeepsTheNewSelection() async throws {
        let store = MemoryPreferences()
        let preferences = ChatPreferences(store: store)
        let recorded = RecordedTransport(try .named("default-model-refused-http"))
        let (ready, release) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        defer { release.finish() }
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                    transport: DelayedResponse(recorded: recorded, ready: ready))
        let thread = ChatThread(library: try await ChatLibrary.reactUI(), gateway: gateway, preferences: preferences)
        thread.send("Use the old selection")
        try #require(await eventually { recorded.sent.count == 1 })
        preferences.selectModel("openai/gpt-5.4-mini")

        release.yield(())
        await thread.waitUntilIdle()
        #expect(thread.error == "The model anthropic/claude-sonnet-5.5 does not exist.")
        #expect(ChatPreferences(store: store).modelID == "openai/gpt-5.4-mini")
    }

    @Test(arguments: [
        ("default-model-refused-http", "The model anthropic/claude-sonnet-5.5 does not exist."),
        ("default-model-refused-sse", "Unknown model: anthropic/claude-sonnet-5.5"),
    ])
    func aLateDefaultRefusalKeepsAnExplicitReselection(recording: String, message: String) async throws {
        let store = MemoryPreferences()
        let preferences = ChatPreferences(store: store)
        let recorded = RecordedTransport(try .named(recording))
        let (ready, release) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        defer { release.finish() }
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                    transport: DelayedResponse(recorded: recorded, ready: ready))
        let thread = ChatThread(library: try await ChatLibrary.reactUI(), gateway: gateway, preferences: preferences)
        thread.send("Use the old selection")
        try #require(await eventually { recorded.sent.count == 1 })
        #expect(try JSONValue.parse(recorded.sent[0].bodyText)["model"]?.string == Template.defaultModel)
        preferences.selectModel("openai/gpt-5.4-mini")
        preferences.selectModel(Template.defaultModel)

        release.yield(())
        await thread.waitUntilIdle()
        #expect(thread.error == message)
        #expect(preferences.modelID == Template.defaultModel)
        #expect(ChatPreferences(store: store).modelID == Template.defaultModel)
    }
}

private struct DelayedResponse: HTTPTransport {
    let recorded: RecordedTransport
    let ready: AsyncStream<Void>

    func send(_ request: URLRequest) async throws -> HTTPResponse {
        let response = try await recorded.send(request)
        var iterator = ready.makeAsyncIterator()
        _ = await iterator.next()
        return response
    }
}
