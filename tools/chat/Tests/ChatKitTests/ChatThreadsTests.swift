import Foundation
import OpenUINative
import Testing
import Synchronization
@testable import ChatKit

@MainActor struct ChatThreadsTests {
    @Test func aFreshWorkspaceRebuildsTheThreadFromStoredItemsAndContinuesIt() async throws {
        let sent = try RecordedTransport(.named("workspace-answer"), .named("threads-token"),
                                        .named("workspace-created"), .named("threads-appended"))
        let original = try await workspace(sent)
        #expect(original.thread.send("First"))
        await original.thread.waitUntilIdle()
        #expect(original.selectedID == "created")
        #expect(original.thread.error == nil)
        #expect(sent.sent.map { $0.url?.path } == [
            "/v1/embed/chat/completions", "/v1/frontend-tokens", "/v1/conversations", "/v1/conversations/created/items",
        ])
        let append = try #require(sent.sent.last)
        let firstTurn = try JSONValue.parse(append.bodyText)["items"]?.array
        #expect(firstTurn?.map { $0["content"]?.array?.first?["text"]?.string } == ["First", recordedAnswer])

        // No transport, in-memory state or thread from the first run is reused.
        let received = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
                                            .named("workspace-items"), .named("workspace-answer"),
                                            .named("threads-appended"))
        let reopened = try await workspace(received)
        await reopened.loadThreads()
        #expect(reopened.threads.map(\.title) == ["First", "Older"])
        #expect(reopened.thread.turns.isEmpty)
        await reopened.open("created")

        #expect(reopened.selectedID == "created")
        let drawn = try #require(assistant(in: reopened.thread))
        #expect(drawn.rawText == recordedAnswer)
        #expect(drawn.live.root?.typeName == "Stack")
        #expect(!drawn.live.isStreaming)
        #expect(drawn.drawFailure == nil)
        let click = try await recordedClick()
        #expect(reopened.thread.send(click))
        await reopened.thread.waitUntilIdle()
        #expect(reopened.thread.error == nil)
        let generation = try #require(received.sent.dropLast().last)
        let messages = try JSONValue.parse(generation.bodyText)["messages"]?.array
        #expect(messages?.dropFirst().map { $0["content"]?.string } == ["First", recordedAnswer, click])
        let secondAppend = try #require(received.sent.last)
        let expected = try Recording.named("threads-append-request").chunks.joined()
        #expect(Array(secondAppend.bodyText.utf8) == Array(expected.utf8))
    }

    @Test func sidebarIsNewestFirstAndRenameDeleteAndNewChatUseTheSelectedThread() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
                                             .named("workspace-items"), .named("threads-renamed"),
                                             .named("threads-deleted"))
        let chats = try await workspace(transport)
        await chats.loadThreads()
        #expect(chats.threads.map(\.id) == ["created", "older"])
        await chats.open("created")
        let current = chats.thread
        current.draft = "An unsent draft"
        await chats.rename("created", to: "  \n Renamed  ")
        #expect(chats.threads.first?.title == "Renamed")
        #expect(chats.thread === current)
        #expect(chats.thread.draft == "An unsent draft")

        await chats.delete("created")
        #expect(chats.threads.map(\.id) == ["older"])
        #expect(chats.selectedID == nil)
        #expect(chats.thread.turns.isEmpty)
        #expect(chats.thread.draft.isEmpty)
        let afterDelete = chats.thread
        afterDelete.draft = "Another unsent draft"
        await chats.newChat()
        #expect(chats.thread !== afterDelete)
        #expect(chats.thread.draft.isEmpty)
        #expect(transport.sent.map(\.httpMethod) == ["POST", "GET", "GET", "POST", "DELETE"])
        #expect(transport.sent.dropFirst(3).first?.bodyText == #"{"title":"Renamed"}"#)
    }

    @Test func failedListingShowsTheCalloutAndLeavesNewChatAvailable() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("threads-list-failed"))
        let chats = try await workspace(transport)
        await chats.loadThreads()
        #expect(chats.storageError == "Storage refused [your frontend token].")
        #expect(chats.threads.isEmpty)
        #expect(chats.selectedID == nil)
        #expect(!chats.isBusy)
        await chats.newChat()
        #expect(chats.thread.turns.isEmpty)
        #expect(transport.sent.map(\.httpMethod) == ["POST", "GET"])
    }

    @Test func stoppingFinishesTheRecordedAppendBeforeTheThreadCanBeReopened() async throws {
        let transport = try RecordedTransport(.named("stopped-mid-answer"), .named("threads-token"),
                                             .named("workspace-created"), .named("threads-appended"),
                                             .named("workspace-items"))
        let chats = try await workspace(transport)
        #expect(chats.thread.send("First"))
        #expect(await eventually { transport.holding.isSet && assistant(in: chats.thread)?.rawText.contains("Paris") == true })
        let raw = try #require(assistant(in: chats.thread)).rawText
        await chats.newChat()
        #expect(chats.thread.turns.isEmpty)
        #expect(transport.cancelled.isSet)
        await chats.open("created")
        let append = try #require(transport.sent.dropLast().last)
        let stored = try JSONValue.parse(append.bodyText)["items"]?.array
        #expect(stored?.map { $0["content"]?.array?.first?["text"]?.string } == ["First", raw])
        #expect(transport.sent.last?.httpMethod == "GET")
    }

    @Test func restoredButtonRunsThroughToStorage() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
            .named("workspace-button-items"), .named("workspace-answer"), .named("threads-appended"))
        let chats = try await workspace(transport)
        await chats.loadThreads()
        await chats.open("created")
        let answer = try #require(assistant(in: chats.thread))
        let stack = try #require(answer.live.root)
        guard case let .list(children)? = stack.props.first(where: { $0.name == "children" })?.value,
              case let .element(button)? = children.first else {
            Issue.record("Restored answer did not render its button")
            return
        }
        #expect(button.typeName == "Button")

        try answer.live.run("action", of: button, message: "Next")
        await chats.thread.waitUntilIdle()

        #expect(transport.sent.suffix(2).map { $0.url?.path } ==
            ["/v1/embed/chat/completions", "/v1/conversations/created/items"])
        let append = try #require(transport.sent.last)
        let expected = try Recording.named("threads-append-request").chunks.joined()
        #expect(append.bodyText.utf8.elementsEqual(expected.utf8))
        let marker = try await recordedClick()
        let stored = try #require(try storedText(in: transport)?.first)
        #expect(stored.utf8.elementsEqual(marker.utf8))
    }

    @Test(arguments: ["open", "delete"])
    func refusedSidebarOperationKeepsAnActiveReply(_ operation: String) async throws {
        let finishReply = Flag()
        defer { finishReply.set() }
        var reply = try Recording.named("stopped-mid-answer")
        reply.release = finishReply
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
            .named("workspace-items"), reply, .named("workspace-operation-refused"), .named("threads-appended"))
        let chats = try await workspace(transport)
        await chats.loadThreads()
        await chats.open("created")
        let active = chats.thread
        defer { active.stop() }
        #expect(active.send("Follow up"))
        try #require(await eventually {
            transport.holding.isSet && assistant(in: active)?.rawText.contains("Paris") == true
        })

        if operation == "open" {
            await chats.open("older")
        } else {
            await chats.delete("created")
        }
        #expect(chats.storageError == "Sidebar operation refused.")
        let sameThread = chats.thread === active
        #expect(sameThread)
        #expect(chats.selectedID == "created")
        #expect(active.isRunning)
        #expect(!transport.cancelled.isSet)
        #expect(appends(in: transport).isEmpty)

        finishReply.set()
        await active.waitUntilIdle()
        #expect(active.error == nil)
        #expect(appends(in: transport).count == 1)
    }

    @Test(arguments: [true, false])
    func aReplyThatFinishesDuringDeleteWaitsForTheResult(_ succeeds: Bool) async throws {
        let finishReply = Flag()
        let finishDelete = Flag()
        defer {
            finishReply.set()
            finishDelete.set()
        }
        var reply = try Recording.named("stopped-mid-answer")
        reply.release = finishReply
        var deletion = try Recording.named(succeeds ? "threads-deleted" : "workspace-operation-refused")
        deletion.beforeResponse = finishDelete
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
            .named("workspace-items"), reply, deletion, .named("threads-appended"))
        let chats = try await workspace(transport)
        await chats.loadThreads()
        await chats.open("created")
        let active = chats.thread
        #expect(active.send("Follow up"))
        try #require(await eventually { transport.holding.isSet })

        let deleting = Task { await chats.delete("created") }
        try #require(await eventually { transport.sent.last?.httpMethod == "DELETE" })
        finishReply.set()
        try #require(await eventually { assistant(in: active)?.live.isStreaming == false })
        #expect(appends(in: transport).isEmpty)
        finishDelete.set()
        await deleting.value
        await active.waitUntilIdle()

        if succeeds {
            #expect(chats.selectedID == nil)
            #expect(chats.thread.turns.isEmpty)
            #expect(appends(in: transport).isEmpty)
        } else {
            #expect(chats.selectedID == "created")
            #expect(chats.storageError == "Sidebar operation refused.")
            #expect(try storedText(in: transport)?.first == "Follow up")
        }
    }

    @Test func deletingAnActiveThreadWaitsForAnAlreadyStartedAppend() async throws {
        let finishAppend = Flag()
        defer { finishAppend.set() }
        var append = try Recording.named("threads-appended")
        append.beforeResponse = finishAppend
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
            .named("workspace-items"), .named("workspace-answer"), append, .named("threads-deleted"))
        let chats = try await workspace(transport)
        await chats.loadThreads()
        await chats.open("created")
        #expect(chats.thread.send("Follow up"))
        try #require(await eventually { appends(in: transport).count == 1 })
        let deleting = Task { await chats.delete("created") }
        try #require(await eventually { chats.isBusy })
        #expect(transport.sent.allSatisfy { $0.httpMethod != "DELETE" })

        finishAppend.set()
        await deleting.value
        #expect(chats.selectedID == nil)
        #expect(transport.sent.suffix(2).map(\.httpMethod) == ["POST", "DELETE"])
    }

    @Test func aFailedModelRequestDoesNotCreateOrStoreAThread() async throws {
        let transport = try RecordedTransport(.named("http-error"))
        let chats = try await workspace(transport)
        #expect(chats.thread.send("First"))
        await chats.thread.waitUntilIdle()
        #expect(chats.thread.error == "Invalid API key provided.")
        #expect(chats.selectedID == nil)
        #expect(chats.threads.isEmpty)
        #expect(transport.sent.map { $0.url?.path } == ["/v1/embed/chat/completions"])
    }

    @Test func aGatewayRefusalLeavesTheOpenedThreadAndItsNextRequestUnchanged() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
            .named("workspace-items"), .named("http-error"), .named("workspace-answer"), .named("threads-appended"))
        let chats = try await workspace(transport)
        await chats.loadThreads()
        await chats.open("created")
        #expect(chats.thread.send("Refused"))
        await chats.thread.waitUntilIdle()
        #expect(chats.thread.error == "Invalid API key provided.")
        #expect(chats.thread.turns.count == 2)
        #expect(appends(in: transport).isEmpty)

        #expect(chats.thread.send("Next"))
        await chats.thread.waitUntilIdle()
        #expect(try replay(in: transport) == ["First", recordedAnswer, "Next"])
        #expect(try storedText(in: transport) == ["Next", recordedAnswer])
    }

    @Test func aRefusedCreateDoesNotEnterReplayOrChangeTheNextTitle() async throws {
        let transport = try RecordedTransport(.named("workspace-answer"), .named("threads-token"),
            .named("workspace-operation-refused"), .named("workspace-answer"), .named("workspace-created"),
            .named("threads-appended"))
        let chats = try await workspace(transport)
        #expect(chats.thread.send("First"))
        await chats.thread.waitUntilIdle()
        #expect(chats.selectedID == nil)
        #expect(chats.thread.error?.contains("Sidebar operation refused.") == true)
        #expect(appends(in: transport).isEmpty)

        #expect(chats.thread.send("Second"))
        await chats.thread.waitUntilIdle()
        #expect(chats.thread.error == nil)
        #expect(transport.sent.dropLast().last?.bodyText == #"{"title":"Second"}"#)
        #expect(try replay(in: transport) == ["Second"])
        #expect(try storedText(in: transport) == ["Second", recordedAnswer])
    }

    @Test func refusedFirstAppendDoesNotPublishOrReplayTheTurn() async throws {
        let transport = try RecordedTransport(.named("workspace-answer"), .named("threads-token"),
            .named("workspace-created"), .named("workspace-append-failed"), .named("threads-deleted"),
            .named("workspace-answer"), .named("workspace-created"), .named("threads-appended"))
        let chats = try await workspace(transport)
        #expect(chats.thread.send("First"))
        await chats.thread.waitUntilIdle()
        #expect(chats.selectedID == nil)
        #expect(chats.threads.isEmpty)
        #expect(chats.thread.turns.isEmpty)
        #expect(chats.thread.error == "Could not save this turn. Items could not be stored.")
        #expect(transport.sent.last?.httpMethod == "DELETE")

        #expect(chats.thread.send("Second"))
        await chats.thread.waitUntilIdle()
        #expect(chats.thread.error == nil)
        #expect(try replay(in: transport) == ["Second"])
        #expect(try storedText(in: transport) == ["Second", recordedAnswer])
    }

    @Test func refusedLaterAppendDoesNotBecomeReplayHistory() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
            .named("workspace-items"), .named("workspace-answer"), .named("workspace-append-failed"),
            .named("workspace-answer"), .named("threads-appended"))
        let chats = try await workspace(transport)
        await chats.loadThreads()
        await chats.open("created")
        #expect(chats.thread.send("Refused"))
        await chats.thread.waitUntilIdle()
        #expect(chats.thread.error == "Could not save this turn. Items could not be stored.")
        #expect(chats.thread.turns.count == 2)
        #expect(chats.thread.send("Next"))
        await chats.thread.waitUntilIdle()
        #expect(try replay(in: transport) == ["First", recordedAnswer, "Next"])
        #expect(try storedText(in: transport) == ["Next", recordedAnswer])
    }

    @Test func cleanupFailureKeepsTheEmptyConversationHiddenAndRetriesBeforeTheNextSend() async throws {
        let transport = try RecordedTransport(.named("workspace-answer"), .named("threads-token"),
            .named("workspace-created"), .named("workspace-append-failed"), .named("workspace-operation-refused"),
            .named("threads-deleted"), .named("workspace-answer"), .named("workspace-created"), .named("threads-appended"))
        let chats = try await workspace(transport)
        #expect(chats.thread.send("First"))
        await chats.thread.waitUntilIdle()
        #expect(chats.selectedID == nil)
        #expect(chats.threads.isEmpty)
        #expect(chats.thread.error?.contains("Could not remove the empty conversation.") == true)

        #expect(chats.thread.send("Second"))
        await chats.thread.waitUntilIdle()
        #expect(chats.thread.error == nil)
        #expect(transport.sent.filter { $0.httpMethod == "DELETE" }.count == 2)
        #expect(try replay(in: transport) == ["Second"])
        #expect(try storedText(in: transport) == ["Second", recordedAnswer])
    }

    @Test(arguments: [true, false])
    func ambiguousFirstAppendUsesTheStoredItemsBeforeAnyNewWrite(_ wasStored: Bool) async throws {
        let read = wasStored ? "workspace-items" : "workspace-items-empty"
        let afterRead = wasStored ? "workspace-answer" : "threads-deleted"
        let transport = try RecordedTransport(.named("workspace-answer"), .named("threads-token"),
            .named("workspace-created"), .named("workspace-append-uncertain"), .named(read), .named(afterRead),
            .named(wasStored ? "threads-appended" : "workspace-answer"), .named("workspace-created"), .named("threads-appended"))
        let chats = try await workspace(transport)
        #expect(chats.thread.send("First"))
        await chats.thread.waitUntilIdle()
        #expect(chats.selectedID == (wasStored ? "created" : nil))
        #expect(chats.thread.turns.count == (wasStored ? 2 : 0))
        #expect((chats.thread.error == nil) == wasStored)
        #expect(appends(in: transport).count == 1)

        #expect(chats.thread.send("Next"))
        await chats.thread.waitUntilIdle()
        #expect(chats.thread.error == nil)
        #expect(try replay(in: transport) == (wasStored ? ["First", recordedAnswer, "Next"] : ["Next"]))
        #expect(appends(in: transport).count == 2)
        #expect(try storedText(in: transport) == ["Next", recordedAnswer])
    }

    @Test func failedReconciliationRequiresAnotherReadBeforeGeneration() async throws {
        let transport = try RecordedTransport(.named("workspace-answer"), .named("threads-token"),
            .named("workspace-created"), .named("workspace-append-uncertain"), .named("workspace-operation-refused"),
            .named("workspace-items"), .named("workspace-answer"), .named("threads-appended"))
        let chats = try await workspace(transport)
        #expect(chats.thread.send("First"))
        await chats.thread.waitUntilIdle()
        #expect(chats.thread.error?.contains("Sidebar operation refused.") == true)
        #expect(chats.threads.isEmpty)
        #expect(chats.thread.turns.isEmpty)

        #expect(chats.thread.send("Next"))
        await chats.thread.waitUntilIdle()
        #expect(chats.selectedID == "created")
        #expect(chats.thread.error == nil)
        #expect(transport.sent.suffix(3).map(\.httpMethod) == ["GET", "POST", "POST"])
        #expect(try replay(in: transport) == ["First", recordedAnswer, "Next"])
        #expect(try storedText(in: transport) == ["Next", recordedAnswer])
    }

    @Test func oversizeStoredAnswerIsRefusedBeforeReplay() async throws {
        var oversized = try Recording.named("workspace-oversize-items")
        oversized.chunks = oversized.chunks.map {
            $0.replacingOccurrences(of: "OVERSIZE_FILLER",
                with: String(repeating: "A", count: ThesysGateway.maxLineBytes))
        }
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
            .named("workspace-items"), oversized, .named("workspace-answer"), .named("threads-appended"))
        let chats = try await workspace(transport)
        await chats.loadThreads()
        await chats.open("created")
        let active = chats.thread
        await chats.open("older")
        #expect(chats.storageError == "response bundle exceeds the program-size limit")
        let sameThread = chats.thread === active
        #expect(sameThread)
        #expect(chats.selectedID == "created")
        #expect(chats.thread.send("Next"))
        await chats.thread.waitUntilIdle()
        #expect(try replay(in: transport) == ["First", recordedAnswer, "Next"])
    }

    @Test func credentialChangeCannotSendHistoryUnderDifferentKey() async throws {
        let store = SwitchingKeyStore()
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"),
            .named("workspace-items"), .named("workspace-answer"), .named("threads-appended"))
        let chats = try await workspace(transport, keyStore: store)
        await chats.loadThreads()
        await chats.open("created")
        store.switchAfterNextRead()
        #expect(chats.thread.send("Next"))
        await chats.thread.waitUntilIdle()

        #expect(chats.thread.error == nil)
        #expect(try replay(in: transport) == ["First", recordedAnswer, "Next"])
        #expect(transport.sent.suffix(2).allSatisfy {
            $0.value(forHTTPHeaderField: "Authorization") == "Bearer \(testKey)"
        })
        #expect(appends(in: transport).count == 1)

        let requestCount = transport.sent.count
        #expect(chats.thread.send("Wrong scope"))
        await chats.thread.waitUntilIdle()
        #expect(chats.thread.error == "Your Thesys API key changed. Open your threads again.")
        #expect(transport.sent.count == requestCount)
    }

    private func appends(in transport: RecordedTransport) -> [URLRequest] {
        transport.sent.filter { $0.httpMethod == "POST" && $0.url?.path.hasSuffix("/items") == true }
    }

    private func replay(in transport: RecordedTransport) throws -> [String]? {
        let request = try #require(transport.sent.last { $0.url?.path == "/v1/embed/chat/completions" })
        return try JSONValue.parse(request.bodyText)["messages"]?.array?.dropFirst().compactMap { $0["content"]?.string }
    }

    private func storedText(in transport: RecordedTransport) throws -> [String]? {
        let request = try #require(appends(in: transport).last)
        return try JSONValue.parse(request.bodyText)["items"]?.array?.compactMap {
            $0["content"]?.array?.first?["text"]?.string
        }
    }

    private func workspace(_ transport: RecordedTransport, keyStore: any APIKeyStore = InMemoryKeyStore(testKey))
        async throws -> ChatThreads {
        return ChatThreads(library: try await ChatLibrary.reactUI(),
                           gateway: ThesysGateway(keyStore: keyStore, transport: transport),
                           storage: ThesysConversations(keyStore: keyStore, transport: transport))
    }

    private func assistant(in thread: ChatThread) -> AssistantTurn? {
        if case let .assistant(answer)? = thread.turns.last { answer } else { nil }
    }
}


// The read that validates the old scope returns its credential but changes
// what any subsequent read sees, just as a Settings save between requests can.
private final class SwitchingKeyStore: APIKeyStore {
    private let state = Mutex((key: Optional(testKey), switchOnRead: false))
    func switchAfterNextRead() { state.withLock { $0.switchOnRead = true } }
    func hasKey() -> Bool { state.withLock { $0.key != nil } }
    func read() -> String? {
        state.withLock {
            let key = $0.key
            if $0.switchOnRead {
                $0.key = "different-test-key"
                $0.switchOnRead = false
            }
            return key
        }
    }
    func save(_ key: String) { state.withLock { $0.key = key } }
    func delete() { state.withLock { $0.key = nil } }
}
