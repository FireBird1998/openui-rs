import CryptoKit
import Foundation
import OpenUINative

struct Conversation: Identifiable, Equatable, Sendable {
    let id: String
    let title: String
    let createdAt: Double

    init?(_ json: JSONValue) {
        guard let id = json["id"]?.string, !id.isEmpty, id != ".", id != "..",
              let createdAt = json["created_at"]?.number, createdAt.isFinite else { return nil }
        self.id = id
        self.title = json["title"]?.string ?? "New conversation"
        self.createdAt = createdAt
    }
}

/// A credential validated against the workspace's account, kept for one operation.
struct ConversationCredential: Sendable {
    let key: String
    fileprivate init(_ key: String) { self.key = key }
}

/// The template's browser side uses a scoped frontend token. Its server
/// writes items with the key. This client keeps both calls in the app.
actor ThesysConversations {
    static let appID = "openui-mac-chat"
    static let userID = "demo-user"
    private static let frontendHeader = "x-thesys-frontend-token"
    private static let pageLimit = 100
    private static let maxItemPages = 50

    private struct Token {
        let value: String
        let expiresAt: Double
    }

    private let keyStore: any APIKeyStore
    private let transport: any HTTPTransport
    private var token: Token?
    // A client's conversation IDs belong to the account that first used it.
    // Settings swaps the client on credential changes.
    private var keyScope: SHA256.Digest?

    init(keyStore: any APIKeyStore = KeychainKeyStore(), transport: any HTTPTransport = URLSessionTransport()) {
        self.keyStore = keyStore
        self.transport = transport
    }

    func list() async throws -> [Conversation] {
        let response = try await frontend("GET", path: "/v1/conversations", query: "limit=\(Self.pageLimit)")
        return try Self.listData(response).compactMap { item in
            guard Self.belongsToApp(item) else { return nil }
            return Conversation(item)
        }
    }

    func create(firstMessage: String, credential: ConversationCredential? = nil) async throws -> Conversation {
        let text = Self.trim(firstMessage)
        let title = text.isEmpty ? "New conversation" : String(decoding: text.utf16.prefix(60), as: UTF16.self)
        let response = try await frontend("POST", path: "/v1/conversations", body: .object([("title", .string(title))]),
                                          credential: credential)
        return try Self.conversation(response)
    }

    func rename(id: String, title: String) async throws -> Conversation {
        let response = try await frontend("POST", path: Self.path(id), body: .object([("title", .string(title))]))
        let conversation = try Self.conversation(response)
        guard conversation.id == id else { throw Self.invalidResponse }
        return conversation
    }

    func delete(id: String, credential: ConversationCredential? = nil) async throws {
        _ = try await frontend("DELETE", path: Self.path(id), expectsJSON: false, credential: credential)
    }

    func messages(id: String, credential: ConversationCredential? = nil) async throws -> [ChatMessage] {
        let credential = try credential ?? scopedCredential()
        let path = try Self.path(id) + "/items"
        let query = "order=asc&limit=\(Self.pageLimit)"
        var cursor: String?
        var seen: Set<String> = []
        var messages: [ChatMessage] = []
        for _ in 0..<Self.maxItemPages {
            let after = try cursor.map { "&after=" + (try Self.encode($0)) } ?? ""
            let response = try await frontend("GET", path: path, query: query + after, credential: credential)
            messages.append(contentsOf: try Self.listData(response).compactMap(Self.message))
            guard response["has_more"]?.bool == true else { return messages }
            guard let next = response["last_id"]?.string, !next.isEmpty, seen.insert(next).inserted else {
                throw Self.invalidResponse
            }
            cursor = next
        }
        throw Self.invalidResponse
    }

    func append(id: String, messages: [ChatMessage], credential: ConversationCredential? = nil) async throws {
        let items: [JSONValue] = messages.compactMap { message in
            guard message.role != .system, !message.content.isEmpty else { return nil }
            let part: JSONValue = .object([
                ("type", .string(message.role == .user ? "input_text" : "output_text")),
                ("text", .string(message.content)),
            ])
            return .object([("type", .string("message")), ("role", .string(message.role.rawValue)),
                            ("content", .array([part]))])
        }
        guard !items.isEmpty else { return }
        let key = try (credential ?? scopedCredential()).key
        _ = try await request("POST", path: Self.path(id) + "/items", body: .object([("items", .array(items))]),
                              key: key, expectsJSON: false)
    }

    // String.trim() uses ECMAScript whitespace, and the browser counts UTF-16
    // units when it derives a title.
    private static let titleWhitespace = CharacterSet(charactersIn:
        "\u{0009}\u{000A}\u{000B}\u{000C}\u{000D}\u{0020}\u{00A0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}"
        + "\u{2006}\u{2007}\u{2008}\u{2009}\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}")

    static func trim(_ text: String) -> String {
        text.trimmingCharacters(in: titleWhitespace)
    }

    private static let invalidResponse = GatewayError("Invalid response from Thesys Conversations.")

    private static func belongsToApp(_ json: JSONValue) -> Bool {
        guard let appID = json["app_id"]?.string else { return true } // The token already scopes the list.
        return appID == Self.appID
    }

    private static func conversation(_ json: JSONValue) throws -> Conversation {
        guard belongsToApp(json), let conversation = Conversation(json) else { throw invalidResponse }
        return conversation
    }

    private static func listData(_ json: JSONValue) throws -> [JSONValue] {
        guard let data = json["data"]?.array else { throw invalidResponse }
        return data
    }

    private static func message(_ item: JSONValue) -> ChatMessage? {
        guard item["type"]?.string == "message",
              let role = ChatMessage.Role(rawValue: item["role"]?.string ?? "user"), role != .system else { return nil }
        let content = item["content"]
        let text = content?.string ?? (content?.array ?? []).compactMap { part -> String? in
            switch part["type"]?.string {
            case "input_text", "output_text", "text", "summary_text", "reasoning_text": return part["text"]?.string
            case "refusal": return part["refusal"]?.string.map { "[Refusal]: " + $0 }
            default: return nil
            }
        }.joined()
        return ChatMessage(role, text)
    }

    func scopedCredential() throws -> ConversationCredential {
        let key = try ThesysGateway.key(from: keyStore)
        let digest = SHA256.hash(data: Data(key.utf8))
        guard keyScope == nil || keyScope == digest else {
            throw GatewayError("Your Thesys API key changed. Open your threads again.")
        }
        keyScope = digest
        return ConversationCredential(key)
    }

    private func frontend(_ method: String, path: String, query: String? = nil, body: JSONValue? = nil,
                          expectsJSON: Bool = true, credential: ConversationCredential? = nil) async throws -> JSONValue {
        let key = try (credential ?? scopedCredential()).key
        let value = try await frontendToken(key: key)
        do {
            return try await request(method, path: path, query: query, body: body, key: key, token: value,
                                     expectsJSON: expectsJSON)
        } catch let error as GatewayError where error.status == 401 {
            if token?.value == value { token = nil }
            let fresh = try await frontendToken(key: key)
            return try await request(method, path: path, query: query, body: body, key: key, token: fresh,
                                     expectsJSON: expectsJSON)
        }
    }

    private func frontendToken(key: String) async throws -> String {
        if let token, Date().timeIntervalSince1970 < token.expiresAt - 60 { return token.value }
        let body: JSONValue = .object([("user_id", .string(Self.userID)), ("app_id", .string(Self.appID))])
        let response = try await request("POST", path: "/v1/frontend-tokens", body: body, key: key)
        guard let value = response["token"]?.string, !value.isEmpty, let expiry = response["expires_at"]?.number,
              expiry.isFinite else { throw Self.invalidResponse }
        token = Token(value: value, expiresAt: expiry)
        return value
    }

    private static func encode(_ value: String) throws -> String {
        let allowed = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()")
        guard let encoded = value.addingPercentEncoding(withAllowedCharacters: allowed) else {
            throw invalidResponse
        }
        return encoded
    }

    private static func path(_ id: String) throws -> String {
        guard !id.isEmpty, id != ".", id != ".." else { throw invalidResponse }
        return "/v1/conversations/" + (try encode(id))
    }

    private nonisolated func request(_ method: String, path: String, query: String? = nil, body: JSONValue? = nil,
                                     key: String, token: String? = nil, expectsJSON: Bool = true) async throws -> JSONValue {
        var url = URLComponents()
        url.scheme = "https"
        url.host = "api.thesys.dev"
        url.percentEncodedPath = path
        url.percentEncodedQuery = query
        guard let endpoint = url.url else { throw Self.invalidResponse }
        var request = URLRequest(url: endpoint, cachePolicy: .reloadIgnoringLocalCacheData)
        request.httpMethod = method
        if let token {
            request.setValue(token, forHTTPHeaderField: Self.frontendHeader)
        } else {
            request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        }
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = Data(body.compact.utf8)
        }

        do {
            let response = try await transport.send(request)
            guard (200..<300).contains(response.status) else {
                throw await ThesysGateway.refusal(status: response.status, body: response.body)
            }
            // Releasing the iterator also ends a body we don't need.
            guard expectsJSON else {
                withExtendedLifetime(response.body.makeAsyncIterator()) {}
                return .null
            }
            var data: [UInt8] = []
            for try await byte in response.body {
                guard data.count < defaultHostByteLimit else {
                    throw OpenUIError.hostInputByteLimitExceeded
                }
                data.append(byte)
            }
            try Task.checkCancellation()
            guard let result = try? JSONValue.parse(String(decoding: data, as: UTF8.self)) else {
                throw Self.invalidResponse
            }
            return result
        } catch let error as OpenUIError {
            throw error
        } catch {
            let shown = ThesysGateway.shown(error, hiding: key)
            guard let token, let failure = shown as? GatewayError else { throw shown }
            throw GatewayError(failure.message.replacingOccurrences(of: token, with: "[your frontend token]"),
                               status: failure.status)
        }
    }
}
