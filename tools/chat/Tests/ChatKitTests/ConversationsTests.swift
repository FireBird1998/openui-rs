import Foundation
import OpenUINative
import Testing
@testable import ChatKit

struct ConversationsTests {
    @Test func sidebarUsesTheAppScopedTokenAndTemplateRoutes() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("threads-list"),
                                             .named("threads-created"), .named("threads-renamed"),
                                             .named("threads-deleted"))
        let storage = ThesysConversations(keyStore: InMemoryKeyStore(testKey), transport: transport)
        let threads = try await storage.list()
        let created = try await storage.create(firstMessage: "  " + String(repeating: "ab", count: 40) + "\n")
        let renamed = try await storage.rename(id: created.id, title: "Renamed")
        try await storage.delete(id: created.id)

        #expect(threads.map(\.id) == ["older", "newer"])
        #expect(threads.map(\.title) == ["Older", "Newer"])
        #expect(created.title == String(repeating: "ab", count: 30))
        #expect(renamed.title == "Renamed")
        let requests = transport.sent
        #expect(requests.map(\.httpMethod) == ["POST", "GET", "POST", "POST", "DELETE"])
        #expect(requests.map { $0.url?.absoluteString } == [
            "https://api.thesys.dev/v1/frontend-tokens",
            "https://api.thesys.dev/v1/conversations?limit=100",
            "https://api.thesys.dev/v1/conversations",
            "https://api.thesys.dev/v1/conversations/created",
            "https://api.thesys.dev/v1/conversations/created"
        ])
        let mint = try #require(requests.first)
        #expect(mint.value(forHTTPHeaderField: "Authorization") == "Bearer \(testKey)")
        #expect(mint.value(forHTTPHeaderField: "x-thesys-frontend-token") == nil)
        #expect(mint.bodyText == #"{"user_id":"demo-user","app_id":"openui-mac-chat"}"#)
        for request in requests.dropFirst() {
            #expect(request.value(forHTTPHeaderField: "x-thesys-frontend-token") == "recorded-token")
            #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
        }
        #expect(requests.dropFirst(2).first?.bodyText == #"{"title":"\#(String(repeating: "ab", count: 30))"}"#)
        #expect(requests.dropFirst(3).first?.bodyText == #"{"title":"Renamed"}"#)
        #expect(requests.last?.httpBody == nil)
    }

    @Test func readPagesInAscendingOrderAndAppendOnlyRawTurnItems() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("threads-items-first"),
                                             .named("threads-items-last"), .named("threads-appended"))
        let storage = ThesysConversations(keyStore: InMemoryKeyStore(testKey), transport: transport)
        let messages = try await storage.messages(id: "thread/a?#")
        let action = try await recordedClick()
        #expect(messages == [.init(.user, "First"), .init(.assistant, recordedAnswer), .init(.user, action)])

        try await storage.append(id: "thread/a?#", messages: [.init(.user, action), .init(.assistant, recordedAnswer)])
        let requests = transport.sent
        #expect(requests.dropFirst().map { $0.url?.absoluteString } == [
            "https://api.thesys.dev/v1/conversations/thread%2Fa%3F%23/items?order=asc&limit=100",
            "https://api.thesys.dev/v1/conversations/thread%2Fa%3F%23/items?order=asc&limit=100&after=part%2B1",
            "https://api.thesys.dev/v1/conversations/thread%2Fa%3F%23/items"
        ])
        for request in requests.dropFirst().dropLast() {
            #expect(request.httpMethod == "GET")
            #expect(request.value(forHTTPHeaderField: "x-thesys-frontend-token") == "recorded-token")
        }
        let append = try #require(requests.last)
        #expect(append.httpMethod == "POST")
        #expect(append.value(forHTTPHeaderField: "x-thesys-frontend-token") == nil)
        #expect(append.value(forHTTPHeaderField: "Authorization") == "Bearer \(testKey)")
        let expected = try Recording.named("threads-append-request").chunks.joined()
        #expect(Array(append.bodyText.utf8) == Array(expected.utf8))
    }

    @Test func failedListIsReportedWithoutEchoingTheToken() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("threads-list-failed"))
        let storage = ThesysConversations(keyStore: InMemoryKeyStore(testKey), transport: transport)
        await #expect(throws: GatewayError("Storage refused [your frontend token].", status: 503)) {
            try await storage.list()
        }
        #expect(transport.sent.map(\.httpMethod) == ["POST", "GET"])
    }

    @Test func aRejectedFrontendTokenIsRefreshedOnceAndTheListIsRepeated() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("threads-token-rejected"),
                                             .named("threads-token-refreshed"), .named("threads-list"))
        let client = ThesysConversations(keyStore: InMemoryKeyStore(testKey), transport: transport)
        #expect(try await client.list().map(\.id) == ["older", "newer"])
        #expect(transport.sent.map(\.httpMethod) == ["POST", "GET", "POST", "GET"])
        #expect(transport.sent.map { $0.value(forHTTPHeaderField: "x-thesys-frontend-token") }
                == [nil, "recorded-token", nil, "recorded-refreshed-token"])
    }

    @Test func oversizedStorageReadReportsTheHostInputByteLimit() async throws {
        let oversized = Recording(status: 200, chunks: [String(repeating: " ", count: defaultHostByteLimit) + "{}"])
        let transport = try RecordedTransport(.named("threads-token"), oversized)
        let storage = ThesysConversations(keyStore: InMemoryKeyStore(testKey), transport: transport)
        await #expect(throws: OpenUIError(.limitExceeded, "host input byte limit")) {
            _ = try await storage.list()
        }
    }

    @Test func anOldClientRefusesToWriteUnderAChangedKey() async throws {
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-created"))
        let keyStore = InMemoryKeyStore(testKey)
        let storage = ThesysConversations(keyStore: keyStore, transport: transport)
        let thread = try await storage.create(firstMessage: "First")
        keyStore.save("different-test-key")
        await #expect(throws: GatewayError("Your Thesys API key changed. Open your threads again.")) {
            try await storage.append(id: thread.id, messages: [.init(.user, "First")])
        }
        #expect(transport.sent.count == 2)
    }
}

let recordedAnswer = "]]>openui:content\nroot = Stack([TextContent(\"Recorded\")])\n\n]]>openui:end"

func recordedClick() async throws -> String {
    let message = try await Core().buildActionUserMessage(label: "Next", context: .number(0))
    let repo = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        .appending(path: "../../../..", directoryHint: .isDirectory)
    let url = repo.appending(path: "conformance/golden/action-messages.json")
    let golden = try JSONValue.parse(String(contentsOf: url, encoding: .utf8))
    let expected = try #require(golden["context"]?["message"]?.string)
    #expect(Array(message.utf8) == Array(expected.utf8))
    return message
}
