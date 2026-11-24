import AppKit
import Foundation
import OpenUINative
import SwiftUI
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

private func deltaEvent(_ text: String) -> String {
    let event = JSONValue.object([("choices", .array([.object([("delta", .object([("content", .string(text))]))])]))])
    return "data: " + event.compact + "\n\n"
}

@MainActor private func assistant(_ turn: ChatThread.Turn?) -> AssistantTurn? {
    if case let .assistant(answer)? = turn { answer } else { nil }
}

private func elements(in prop: String, of element: Element) -> [Element] {
    switch element.props.first(where: { $0.name == prop })?.value {
    case let .element(child): [child]
    case let .list(children): children.compactMap {
        if case let .element(child) = $0 { child } else { nil }
    }
    default: []
    }
}

@MainActor private func bookingForm(in answer: AssistantTurn) throws -> (seat: Element, note: Element, save: Element) {
    let root = try #require(answer.live.root)
    let form = try #require(elements(in: "children", of: root).first { $0.typeName == "Form" })
    let fields = elements(in: "fields", of: form)
    let seat = try #require(fields.first.flatMap { elements(in: "input", of: $0).first })
    let note = try #require(fields.dropFirst().first.flatMap { elements(in: "input", of: $0).first })
    let buttons = try #require(elements(in: "buttons", of: form).first)
    let save = try #require(elements(in: "buttons", of: buttons).first)
    return (seat, note, save)
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
        guard case let .user(turn)? = chat.turns.first else { Issue.record("no user turn"); return }
        #expect(turn.text == "What is the capital of France?")
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

    @Test func clickingADrawnButtonSendsTheActionAndDrawsTheReply() async throws {
        let transport = RecordedTransport(try .named("action-controls"), try .named("streamed-answer"))
        let chat = try await thread(transport)
        chat.send("Show booking controls")
        await chat.waitUntilIdle()
        let answer = try #require(assistant(chat.turns.last))
        let root = try #require(answer.live.root)
        let button = try #require(elements(in: "children", of: root).first)
        #expect(button.typeName == "Button")

        try answer.live.run("action", of: button, message: "Continue")
        await chat.waitUntilIdle()

        #expect(chat.error == nil)
        #expect(chat.turns.count == 4)
        let second = try #require(transport.sent.dropFirst().first)
        let action = try #require(try messages(of: second).last?["content"]?.string)
        let expected = "]]>openui:content\nContinue\n]]>openui:context\n[\"User clicked: Continue\",{}]"
        #expect(action.utf8.elementsEqual(expected.utf8))
        guard case let .user(turn)? = chat.turns.dropLast().last else { Issue.record("no action user turn"); return }
        #expect(turn.text == "Continue")
        #expect(turn.formData == nil)
        let reply = try #require(assistant(chat.turns.last))
        #expect(drawnTexts(reply.live) == ["Paris", "Paris is the capital of France."])
    }

    @Test(arguments: [
        ("http-error", "Invalid API key provided."),
        ("error-mid-stream", "The model is overloaded. Try again later."),
    ])
    func aFailedActionLeavesTheThreadAndNextRequestUnchanged(recording: String, message: String) async throws {
        try await assertActionRollback(try .named(recording), showing: message)
    }

    @Test func aResponseSizeRefusalLeavesNoActionOrAssistantTurn() async throws {
        let largeDelta = deltaEvent(String(repeating: " ", count: Self.programSize / 2 + 1))
        try await assertActionRollback(Recording(status: 200, chunks: [largeDelta, largeDelta, "data: [DONE]\n\n"]),
                                       showing: "response bundle exceeds the program-size limit")
    }

    @Test func anActionDrawingRefusalReleasesAParkedBodyAndRollsBack() async throws {
        try await assertActionRollback(try .named("stopped-mid-answer"), showing: "injected draw limit",
                                       drawingRefusal: OpenUIError(.limitExceeded, "injected draw limit"))
    }

    private func assertActionRollback(_ refusal: Recording, showing message: String,
                                      drawingRefusal: OpenUIError? = nil) async throws {
        let transport = RecordedTransport(try .named("action-controls"), refusal, try .named("streamed-answer"))
        let chat = try await thread(transport)
        chat.send("Show booking controls")
        await chat.waitUntilIdle()
        let originalIDs = chat.turns.map(\.id)
        let originalHistory = chat.history.map { ($0.role, Data($0.content.utf8)) }
        let answer = try #require(assistant(chat.turns.last))
        let form = try bookingForm(in: answer)
        try answer.live.enter(.string("3A"), into: form.seat, form: "booking")
        chat.refusesDraw = { _ in drawingRefusal }

        try answer.live.run("action", of: form.save, message: "Save", form: "booking")
        let settled = await eventually { !chat.isRunning }
        #expect(settled)
        if !settled { chat.stop() }
        await chat.waitUntilIdle()

        #expect(chat.error == message)
        #expect(chat.turns.map(\.id) == originalIDs)
        #expect(chat.history.map(\.role) == originalHistory.map(\.0))
        #expect(chat.history.map { Data($0.content.utf8) } == originalHistory.map(\.1))
        #expect(answer.live.field(form: "booking", name: "seat") == .string("3A"))
        if drawingRefusal != nil { #expect(transport.cancelled.isSet) }

        chat.refusesDraw = nil
        let firstAttempt = try #require(transport.sent.last?.httpBody)
        try answer.live.run("action", of: form.save, message: "Save", form: "booking")
        await chat.waitUntilIdle()
        #expect(chat.error == nil)
        #expect(transport.sent.count == 3)
        let nextRequest = try #require(transport.sent.last?.httpBody)
        #expect(nextRequest == firstAttempt)
    }

    @Test func actionAnswerFinishesAtDoneEvenIfTransportDoesNotClose() async throws {
        var parked = try Recording.named("streamed-answer")
        parked.hold = true
        let transport = RecordedTransport(try .named("action-controls"), parked, try .named("streamed-answer"))
        let chat = try await thread(transport)
        chat.send("Show booking controls")
        await chat.waitUntilIdle()
        let answer = try #require(assistant(chat.turns.last))
        let root = try #require(answer.live.root)
        let button = try #require(elements(in: "children", of: root).first)
        try answer.live.run("action", of: button, message: "Continue")

        let settled = await eventually { !chat.isRunning }
        #expect(settled)
        if !settled { chat.stop() }
        await chat.waitUntilIdle()
        #expect(transport.holding.isSet)
        #expect(transport.cancelled.isSet)
        #expect(chat.error == nil)
        #expect(chat.turns.count == 4)
        let reply = try #require(assistant(chat.turns.last))
        #expect(drawnTexts(reply.live) == ["Paris", "Paris is the capital of France."])

        try answer.live.run("action", of: button, message: "Continue")
        await chat.waitUntilIdle()
        #expect(transport.sent.count == 3)
        #expect(chat.turns.count == 6)
        #expect(chat.error == nil)
    }

    @Test func stoppingAnActionKeepsItsPartialReply() async throws {
        let transport = RecordedTransport(try .named("action-controls"), try .named("stopped-mid-answer"))
        let chat = try await thread(transport)
        chat.send("Show booking controls")
        await chat.waitUntilIdle()
        let answer = try #require(assistant(chat.turns.last))
        let root = try #require(answer.live.root)
        let button = try #require(elements(in: "children", of: root).first)
        try answer.live.run("action", of: button, message: "Continue")
        #expect(await eventually { transport.holding.isSet && assistant(chat.turns.last)?.rawText.contains("Paris") == true })
        chat.stop()
        await chat.waitUntilIdle()

        #expect(transport.cancelled.isSet)
        #expect(chat.error == nil)
        #expect(chat.turns.count == 4)
        let reply = try #require(assistant(chat.turns.last))
        #expect(drawnTexts(reply.live) == ["Paris"])
        #expect(reply.live.acceptsInteractions)
    }

    @Test func loneSurrogateInActionContextMatchesPinnedWire() async throws {
        let program = #"root = Stack([Button("Save", @ToAssistant("Save", {x: "\ud800"}))])"#
        let transport = RecordedTransport(Recording(status: 200, chunks: [deltaEvent(program), "data: [DONE]\n\n"]),
                                          try .named("streamed-answer"))
        let chat = try await thread(transport)
        chat.send("Show the button")
        await chat.waitUntilIdle()
        let answer = try #require(assistant(chat.turns.last))
        let root = try #require(answer.live.root)
        let button = try #require(elements(in: "children", of: root).first)
        try answer.live.run("action", of: button, message: "Save")
        await chat.waitUntilIdle()

        #expect(chat.error == nil)
        let sent = try #require(transport.sent.dropFirst().first)
        let content = try #require(try messages(of: sent).last?["content"]?.string)
        let replacement = "\u{FFFD}"
        let expected = "]]>openui:content\nSave\n]]>openui:context\n[\"User clicked: Save\",{},{\"x\":\"\(replacement)\"}]"
        #expect(content.utf8.elementsEqual(expected.utf8))
        let body = try #require(sent.httpBody)
        let wireContext = #"\"x\":\""# + replacement + #"\"}]"}"#
        #expect(body.range(of: Data(wireContext.utf8)) != nil)
    }

    @Test func submittingAnEarlierAnswersFormSendsTheWholeRecordedThreadAndDrawsTheReply() async throws {
        let transport = RecordedTransport(try .named("action-controls"), try .named("streamed-answer"),
                                          try .named("streamed-answer"))
        let chat = try await thread(transport)
        chat.send("Show booking controls")
        await chat.waitUntilIdle()
        let first = try #require(assistant(chat.turns.last))
        let form = try bookingForm(in: first)
        try first.live.enter(.string("3A"), into: form.seat, form: "booking")
        try first.live.enter(.string(""), into: form.note, form: "booking")
        let root = try #require(first.live.root)
        let button = try #require(elements(in: "children", of: root).first)
        try first.live.run("action", of: button, message: "Continue")
        await chat.waitUntilIdle()

        #expect(first.live.acceptsInteractions)
        #expect(first.live.field(form: "booking", name: "seat") == .string("3A"))
        try first.live.run("action", of: form.save, message: "Save", form: "booking")
        await chat.waitUntilIdle()

        #expect(chat.error == nil)
        #expect(chat.turns.count == 6)
        #expect(transport.sent.count == 3)
        let request = try #require(transport.sent.dropFirst(2).first)
        let actual = try messages(of: request)
        let expectedURL = try #require(Bundle.module.url(forResource: "action-thread-request", withExtension: "json",
                                                           subdirectory: "Recordings"))
        let expected = try #require(JSONValue.parse(String(contentsOf: expectedURL, encoding: .utf8))["messages"]?.array)
        #expect(actual.first?["role"]?.string == "system")
        #expect(actual.first?["content"]?.string == (try await Shared.reactUI().systemPrompt))
        #expect(actual.dropFirst().map { $0["role"]?.string } == expected.map { $0["role"]?.string })
        #expect(actual.dropFirst().map { $0["content"]?.string.map { Array($0.utf8) } } ==
                expected.map { $0["content"]?.string.map { Array($0.utf8) } })
        let reply = try #require(assistant(chat.turns.last))
        #expect(reply.id != first.id)
        #expect(drawnTexts(reply.live) == ["Paris", "Paris is the capital of France."])
    }

    @Test func browserOpensOnlyAbsoluteHTTPAndHTTPS() async throws {
        let expected = ["http://example.com/guide", "https://example.com/guide?q=1#forms"]
        let destinations = expected + [
            "/guide", "//example.com/guide", "https:///guide", "http:guide",
            "ftp://example.com/guide", "file:///guide", "javascript:void(0)",
        ]
        let buttons = destinations.map { destination in
            "Button(\"Docs\", @OpenUrl(\(JSONValue.string(destination).compact)))"
        }
        let program = "root = Stack([\(buttons.joined(separator: ", "))])"
        let transport = RecordedTransport(Recording(status: 200, chunks: [deltaEvent(program), "data: [DONE]\n\n"]))
        var opened: [URL] = []
        let chat = ChatThread(library: try await Shared.reactUI(),
                              gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport),
                              openBrowser: { opened.append($0) })
        chat.send("Show booking controls")
        await chat.waitUntilIdle()
        let answer = try #require(assistant(chat.turns.last))
        let root = try #require(answer.live.root)
        let links = elements(in: "children", of: root)
        try #require(links.count == destinations.count)
        for link in links {
            try answer.live.run("action", of: link, message: "Docs")
        }

        #expect(opened.map(\.absoluteString) == expected)
        #expect(chat.turns.count == 2)
        #expect(transport.sent.count == 1)
    }

    @Test func aClickMadeBubbleShowsItsLabelAndOnlyTheNonemptyFormFields() async throws {
        let chat = try await thread(RecordedTransport(try .named("action-controls"), try .named("streamed-answer")))
        chat.send("Show booking controls")
        await chat.waitUntilIdle()
        let answer = try #require(assistant(chat.turns.last))
        let form = try bookingForm(in: answer)
        try answer.live.enter(.string("3A"), into: form.seat, form: "booking")
        try answer.live.enter(.string(""), into: form.note, form: "booking")
        try answer.live.run("action", of: form.save, message: "Save", form: "booking")
        await chat.waitUntilIdle()

        guard case let .user(turn)? = chat.turns.dropLast().last else { Issue.record("no action user turn"); return }
        #expect(turn.text == "Save")
        let formData = try #require(turn.formData)
        #expect(formData.title == "Form data (1 field)")
        #expect(formData.fields.map(\.label) == ["seat"])
        #expect(formData.fields.map(\.value) == ["3A"])
    }

    @Test func aFormDisclosureShowsItsTitleAndRevealsFieldsWhenOpened() async throws {
        let core = Core()
        let raw = try await core.buildActionUserMessage(label: "Save", formState: .object([
            ("seat", .object([("value", .string("3A"))])),
            ("note", .object([("value", .string("Window seat"))])),
        ]))
        let turn = UserTurn(rawText: raw, text: "Save", parsed: try await core.parseMessage(raw))
        _ = NSApplication.shared
        let host = NSHostingView(rootView: UserBubble(turn: turn)
            .environment(\.accessibilityEnabled, true)
            .frame(width: 350)
            .transaction { $0.animation = nil })
        let window = NSWindow(contentRect: CGRect(x: 60, y: 60, width: 350, height: 200),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        host.layoutSubtreeIfNeeded()

        func nodes() -> [AnyObject] {
            var pending: [AnyObject] = [host]
            var result: [AnyObject] = []
            while let node = pending.popLast() {
                result.append(node)
                pending.append(contentsOf: (node.accessibilityChildren?() ?? []).map { $0 as AnyObject })
            }
            return result
        }
        func text(of node: AnyObject) -> [String] {
            let value: (() -> Any?)? = node.accessibilityValue
            return [node.accessibilityLabel?(), node.accessibilityTitle?(), value?() as? String].compactMap { $0 }
        }

        let title = "Form data (2 fields)"
        let fields = ["seat:", "3A", "note:", "Window seat"]
        #expect(await eventually { nodes().contains { text(of: $0).contains(title) } })
        let collapsed = nodes().flatMap(text(of:))
        #expect(collapsed.contains("Save"), "\(collapsed)")
        #expect(fields.allSatisfy { !collapsed.contains($0) }, "\(collapsed)")
        let toggle = try #require(nodes().first { text(of: $0).contains(title) }, "\(collapsed)")
        #expect(toggle.accessibilityPerformPress?() == true)
        let revealed = await eventually {
            host.layoutSubtreeIfNeeded()
            let expanded = nodes().flatMap(text(of:))
            return fields.allSatisfy { expanded.contains($0) }
        }
        #expect(revealed, "\(nodes().flatMap(text(of:)))")
    }

    @Test func aFlatFormUsesTheTemplatesFieldValueText() async throws {
        let core = Core()
        let raw = try await core.buildActionUserMessage(label: "Save", formState: .object([
            ("choice", .object([("value", .array([.string("A"), .null, .string("B")]))])),
            ("checked", .object([("value", .bool(false))])),
            ("numbers", .object([("value", .array([.number(1e20), .number(1e-6), .number(1e-7)]))])),
            ("unset", .object([("value", .null)])),
        ]))
        let turn = UserTurn(rawText: raw, text: "Save", parsed: try await core.parseMessage(raw))
        let formData = try #require(turn.formData)
        #expect(formData.title == "Form data (3 fields)")
        #expect(formData.fields.map(\.label) == ["choice", "checked", "numbers"])
        #expect(formData.fields.map(\.value) == ["A,,B", "false", "100000000000000000000,0.000001,1e-7"])
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
        guard case let .user(turn)? = chat.turns.first else { Issue.record("no user turn"); return }
        #expect(turn.text == "Hello")
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
        // About 64 large deltas reach the limit.
        let transport = EndlessTransport(filler: 16 << 10)
        let chat = ChatThread(library: try await Shared.reactUI(),
                              gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport))
        chat.send("Never stop")
        await chat.waitUntilIdle()
        #expect(!chat.isRunning)
        #expect(chat.error == nil)
        #expect(chat.turns.count == 2)
        guard case let .user(turn)? = chat.turns.first else { Issue.record("no user turn"); return }
        #expect(turn.text == "Never stop")
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

    @Test func anEndlessAnswerInSmallDeltasReachesTheSessionsLimitToo() async throws {
        // Thousands of deltas: each is taken by the session as it arrives,
        // and drawing them does not slow reading down with the answer's
        // length (#206).
        let transport = EndlessTransport()
        let chat = ChatThread(library: try await Shared.reactUI(),
                              gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport))
        chat.send("Never stop")
        await chat.waitUntilIdle()
        #expect(chat.error == nil)
        let answer = try #require(assistant(chat.turns.last))
        #expect(answer.drawFailure == "response bundle exceeds the program-size limit")
        #expect(answer.rawText.utf8.count <= Self.programSize)
        #expect(answer.rawText.utf8.count + transport.filler > Self.programSize)
        #expect(transport.bytesRead.count < Self.programSize * 3 / 2)
        #expect(await eventually { transport.released.isSet })
        #expect(drawnTexts(answer.live) == ["Endless"])
        #expect(answer.live.acceptsInteractions)
    }

    @Test(arguments: [5, 9, 13])
    func aDrawRefusedMidStreamEndsTheAnswerAtTheLastDrawnTextWithNoGap(refusedDraw: Int) async throws {
        let (permits, writer) = AsyncStream<Void>.makeStream()
        let transport = EndlessTransport(permits: permits)
        let chat = ChatThread(library: try await Shared.reactUI(),
                              gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport))
        writer.yield(()) // The head is the first draw.
        chat.refusesDraw = { draw in
            if draw == refusedDraw {
                // Wake a reader waiting for another delta so it observes the refusal.
                writer.yield(())
                writer.finish()
                return OpenUIError(.limitExceeded, "injected draw limit")
            }
            // Pace each statement by its preceding draw. At the last successful
            // draw, let a burst through so an append can overlap the refusal.
            let count = draw == refusedDraw - 1 ? 64 : 1
            for _ in 0..<count { writer.yield(()) }
            return nil
        }
        chat.send("Never stop")
        await chat.waitUntilIdle()
        #expect(chat.error == nil)
        let answer = try #require(assistant(chat.turns.last))
        #expect(answer.drawFailure == "injected draw limit")
        #expect(answer.draws == refusedDraw)
        // Each draw before the refused one showed exactly one more delta.
        let sent = EndlessTransport.head + (1..<(refusedDraw - 1)).map {
            "f\($0) = \"" + String(repeating: "x", count: transport.filler) + "\"\n"
        }.joined()
        #expect(answer.rawText == sent, "\(answer.rawText.split(separator: "\n").map { $0.prefix(6) })")
        #expect(chat.history.last == ChatMessage(.assistant, sent))
        #expect(await eventually { transport.released.isSet })
        #expect(answer.live.acceptsInteractions)
    }

    @Test func aLongAnswerInSmallDeltasIsDrawnAsItsWholeTextIsWithoutADrawPerDelta() async throws {
        let cards = (1...60).map { k in
            "c\(k) = Card([h\(k), t\(k)])\nh\(k) = TextContent(\"Section \(k)\", \"large-heavy\")\n"
                + "t\(k) = TextContent(\"\(k): \(String(repeating: "lorem ipsum ", count: 8))\")\n"
        }
        let text = "root = Stack([" + (1...60).map { "c\($0)" }.joined(separator: ", ") + "])\n" + cards.joined()
        var deltas: [String] = []
        var rest = Substring(text)
        while !rest.isEmpty {
            deltas.append(String(rest.prefix(50)))
            rest = rest.dropFirst(50)
        }
        let events = deltas.map(deltaEvent)
        let chat = try await thread(RecordedTransport(Recording(status: 200, chunks: [events.joined() + "data: [DONE]\n\n"])))
        chat.send("Sixty cards")
        await chat.waitUntilIdle()
        #expect(chat.error == nil)
        let answer = try #require(assistant(chat.turns.last))
        #expect(answer.rawText == text)
        #expect(answer.drawFailure == nil)
        let whole = try LiveProgram(library: try await Shared.reactUI().library, promptSpec: try await Shared.reactUI().promptSpec)
        try await whole.show(response: text, streaming: false)
        #expect(drawnTexts(answer.live) == drawnTexts(whole))
        #expect(drawnTexts(answer.live).count == 120)
        // Pieces that arrive while a draw runs are drawn together by the next.
        #expect(answer.draws > 0 && answer.draws < deltas.count / 2, "\(answer.draws) draws for \(deltas.count) deltas")
        #expect(answer.live.acceptsInteractions)
    }
}
