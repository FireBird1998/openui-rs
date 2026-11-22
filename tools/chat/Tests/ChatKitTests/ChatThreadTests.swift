import Foundation
import OpenUINative
import Testing
@testable import ChatKit

private let repository = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent()

private func conformance(_ path: String) throws -> String {
    try String(contentsOf: repository.appendingPathComponent("conformance/\(path)"), encoding: .utf8)
}

/// One library for the suite: loading it builds the cloud config block.
@MainActor private enum Shared {
    static var library: ChatLibrary?
    static func reactUI() async throws -> ChatLibrary {
        if let library { return library }
        let library = try await ChatLibrary.reactUI()
        Shared.library = library
        return library
    }
}

@MainActor private func thread(_ transport: RecordedTransport, key: String? = testKey) async throws -> ChatThread {
    ChatThread(library: try await Shared.reactUI(),
               gateway: ThesysGateway(keyStore: InMemoryKeyStore(key), transport: transport))
}

/// Every `TextContent`'s text in the drawn tree, in order.
@MainActor private func drawnTexts(_ live: LiveProgram) -> [String] {
    var texts: [String] = []
    var pending: [PropValue] = live.root.map { [.element($0)] } ?? []
    while let value = pending.popLast() {
        switch value {
        case let .element(element):
            if element.typeName == "TextContent", case let .scalar(.string(text))? = element.props.first?.value {
                texts.append(text)
            }
            pending.append(contentsOf: element.props.reversed().map(\.value))
        case let .list(items): pending.append(contentsOf: items.reversed())
        case let .record(fields): pending.append(contentsOf: fields.reversed().map(\.value))
        default: continue
        }
    }
    return texts
}

private func messages(of request: URLRequest) throws -> [JSONValue] {
    try JSONValue.parse(request.bodyText)["messages"]?.array ?? []
}

@MainActor private func assistant(_ turn: ChatThread.Turn?) -> AssistantTurn? {
    if case let .assistant(answer)? = turn { answer } else { nil }
}

@Suite(.serialized) @MainActor struct AChatThread {
    @Test func theLibraryFilesAreTheCorpussReactUILibrary() throws {
        #expect(try ChatLibrary.resource("react-ui-schema") == conformance("imported/schemas/react-ui.json"))
        #expect(try ChatLibrary.resource("react-ui-spec") == conformance("prompts/react-ui-schema.json"))
    }

    @Test func theSystemMessageIsTheTemplatesCloudConfigBlock() async throws {
        let golden = try JSONValue.parse(conformance("golden/prompts/react-ui-cloud.json"))["prompt"]?.string
        #expect(try await Shared.reactUI().systemPrompt == golden)
    }

    @Test func aMessageStreamsAnAnswerThatIsDrawnNatively() async throws {
        let transport = RecordedTransport(try .named("streamed-answer"))
        let chat = try await thread(transport)
        chat.draft = "What is the capital of France?"
        chat.sendDraft()
        #expect(chat.draft.isEmpty)
        #expect(chat.isRunning)
        await chat.waitUntilIdle()

        #expect(!chat.isRunning)
        #expect(chat.error == nil)
        #expect(chat.turns.count == 2)
        guard case let .user(_, text)? = chat.turns.first else { Issue.record("no user turn"); return }
        #expect(text == "What is the capital of France?")
        let answer = try #require(assistant(chat.turns.last))
        #expect(answer.rawText.hasSuffix("Paris is the capital of France.\")])"))
        #expect(answer.live.root?.typeName == "Stack")
        #expect(drawnTexts(answer.live) == ["Paris", "Paris is the capital of France."])
        #expect(answer.live.acceptsInteractions)
        #expect(answer.drawFailure == nil)

        let sent = try messages(of: try #require(transport.sent.first))
        #expect(sent.map { $0["role"]?.string } == ["system", "user"])
        #expect(sent[0]["content"]?.string == (try await Shared.reactUI().systemPrompt))
        #expect(sent[1]["content"]?.string == "What is the capital of France?")
        #expect(try JSONValue.parse(transport.sent[0].bodyText)["model"]?.string == "anthropic/claude-sonnet-5.5")
    }

    @Test func theNextMessageSendsTheWholeThreadWithTheAnswerAsItsRawText() async throws {
        let transport = RecordedTransport(try .named("streamed-answer"), try .named("streamed-answer"))
        let chat = try await thread(transport)
        chat.send("First")
        await chat.waitUntilIdle()
        let raw = try #require(assistant(chat.turns.last)).rawText
        chat.send("Second")
        await chat.waitUntilIdle()
        let sent = try messages(of: transport.sent[1])
        #expect(sent.map { $0["role"]?.string } == ["system", "user", "assistant", "user"])
        #expect(sent.map { $0["content"]?.string }.dropFirst() == ["First", raw, "Second"])
    }

    @Test func stopEndsTheRequestAndLeavesThePartialAnswer() async throws {
        let transport = RecordedTransport(try .named("stopped-mid-answer"))
        let chat = try await thread(transport)
        chat.send("What is the capital of France?")
        #expect(await eventually { transport.holding.isSet && assistant(chat.turns.last)?.rawText.contains("Paris") == true })
        #expect(chat.isRunning)
        // While it runs, a second send does nothing.
        #expect(!chat.send("Another"))

        chat.stop()
        await chat.waitUntilIdle()
        #expect(transport.cancelled.isSet)
        #expect(!chat.isRunning)
        #expect(chat.error == nil)
        #expect(chat.turns.count == 2)
        let answer = try #require(assistant(chat.turns.last))
        #expect(answer.rawText == "root = Stack([title, card])\ntitle = TextContent(\"Paris\", \"large-heavy\")\n")
        #expect(drawnTexts(answer.live) == ["Paris"])
        #expect(answer.drawFailure == nil, "\(answer.drawFailure ?? "")")
        #expect(!answer.live.isStreaming, "requested \(answer.live.isStreamingRequested)")
        #expect(answer.live.acceptsInteractions)
    }

    @Test func anHTTPErrorShowsTheCalloutWithTheGatewaysMessageAndKeepsTheMessage() async throws {
        let chat = try await thread(RecordedTransport(try .named("http-error")))
        chat.draft = "Hello"
        chat.sendDraft()
        await chat.waitUntilIdle()
        #expect(chat.error == "Invalid API key provided.")
        #expect(chat.turns.count == 1)
        guard case let .user(_, text)? = chat.turns.first else { Issue.record("no user turn"); return }
        #expect(text == "Hello")
        #expect(!chat.isRunning)
    }

    @Test func anErrorMidStreamShowsTheCalloutAndKeepsBothMessages() async throws {
        let chat = try await thread(RecordedTransport(try .named("error-mid-stream")))
        chat.send("Hello")
        await chat.waitUntilIdle()
        #expect(chat.error == "The model is overloaded. Try again later.")
        #expect(chat.turns.count == 2)
        let answer = try #require(assistant(chat.turns.last))
        #expect(drawnTexts(answer.live) == ["Paris"])
    }

    @Test func sendingAgainClearsTheErrorAndSendsTheFailedMessageToo() async throws {
        let transport = RecordedTransport(try .named("http-error"), try .named("streamed-answer"))
        let chat = try await thread(transport)
        chat.send("Hello")
        await chat.waitUntilIdle()
        chat.send("Hello again")
        #expect(chat.error == nil)
        await chat.waitUntilIdle()
        #expect(try messages(of: transport.sent[1]).dropFirst().map { $0["content"]?.string } == ["Hello", "Hello again"])
    }

    @Test func withoutAKeyTheCalloutAsksForOne() async throws {
        let transport = RecordedTransport(try .named("streamed-answer"))
        let chat = try await thread(transport, key: nil)
        chat.send("Hello")
        await chat.waitUntilIdle()
        #expect(chat.error == "Add your Thesys API key in Settings.")
        #expect(transport.sent.isEmpty)
    }

    @Test func aBlankMessageIsNotSent() async throws {
        let transport = RecordedTransport(try .named("streamed-answer"))
        let chat = try await thread(transport)
        chat.draft = "  \n "
        chat.sendDraft()
        #expect(chat.turns.isEmpty)
        #expect(chat.draft == "  \n ")
        #expect(!chat.isRunning)
    }

    @Test func anAnswerWithNoProgramDrawsNothing() async throws {
        let prose = "data: {\"choices\":[{\"delta\":{\"content\":\"Just words, no program.\"}}]}\n\ndata: [DONE]\n\n"
        let chat = try await thread(RecordedTransport(Recording(status: 200, chunks: [prose])))
        chat.send("Hello")
        await chat.waitUntilIdle()
        let answer = try #require(assistant(chat.turns.last))
        #expect(answer.rawText == "Just words, no program.")
        #expect(chat.error == nil)
        #expect(drawnTexts(answer.live).isEmpty)
        #expect(answer.drawFailure == nil)
    }

    /// The core's program-size limit (`MAX_PROGRAM_SIZE`, 1 MiB by default),
    /// which the session's response reader applies to the whole response.
    static let programSize = 1 << 20

    @Test func anEndlessAnswerEndsWhenTheSessionRefusesIt() async throws {
        // Each delta is drawn before the next is read, so large deltas keep
        // the test short: about 64 of them reach the limit.
        let transport = EndlessTransport(filler: 16 << 10)
        let chat = ChatThread(library: try await Shared.reactUI(),
                              gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport))
        chat.send("Never stop")
        await chat.waitUntilIdle()
        #expect(!chat.isRunning)
        #expect(chat.error == nil)
        #expect(chat.turns.count == 2)
        guard case let .user(_, text)? = chat.turns.first else { Issue.record("no user turn"); return }
        #expect(text == "Never stop")
        let answer = try #require(assistant(chat.turns.last))
        #expect(answer.drawFailure == "response bundle exceeds the program-size limit")
        // The app's copy is what the session accepted: within its limit, and
        // one more event would have passed it.
        #expect(answer.rawText.utf8.count <= Self.programSize)
        #expect(answer.rawText.utf8.count + transport.filler > Self.programSize)
        // Read no further than the refused event: its text plus each event's
        // JSON around it.
        #expect(transport.bytesRead.count < Self.programSize * 3 / 2)
        #expect(await eventually { transport.released.isSet })
        #expect(drawnTexts(answer.live) == ["Endless"])
        #expect(answer.live.acceptsInteractions)
    }
}
