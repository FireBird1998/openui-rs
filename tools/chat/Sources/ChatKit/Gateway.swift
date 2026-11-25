import Foundation
import OpenUINative

/// One message of a Chat Completions request.
public struct ChatMessage: Sendable, Equatable {
    public enum Role: String, Sendable {
        case system, user, assistant
    }

    public var role: Role
    public var content: String

    public init(_ role: Role, _ content: String) {
        self.role = role
        self.content = content
    }
}

/// What the app asks the Gateway for: a model and the whole thread, the
/// system prompt first.
public struct ChatRequest: Sendable, Equatable {
    public var model: String
    public var messages: [ChatMessage]

    public init(model: String, messages: [ChatMessage]) {
        self.model = model
        self.messages = messages
    }
}

/// Why an answer failed, with the message the person sees under "Something
/// went wrong".
public struct GatewayError: Error, Sendable, Equatable, CustomStringConvertible {
    public let message: String
    /// The HTTP status when the Gateway refused the request; nil for an
    /// error that arrived inside the stream or never reached the Gateway.
    public let status: Int?

    let modelRefused: Bool

    public init(_ message: String, status: Int? = nil) {
        self.init(message, status: status, modelRefused: false)
    }

    init(_ message: String, status: Int?, modelRefused: Bool) {
        self.message = message
        self.status = status
        self.modelRefused = modelRefused
    }

    public var description: String { message }
}

/// The app's one way to the Gateway. `answer` gives the text of the
/// assistant's reply, delta by delta, each read from the network only when
/// it is asked for; it ends when the reply is complete and throws a
/// `GatewayError` when it fails. Cancelling the reading task, or letting the
/// iterator go, stops the request.
public protocol GatewayClient: Sendable {
    func answer(_ request: ChatRequest) -> any AsyncSequence<String, any Error>
}

/// What carries one HTTP exchange: the status and the body as it arrives.
/// The app's is `URLSessionTransport`; tests replay recorded exchanges.
public protocol HTTPTransport: Sendable {
    func send(_ request: URLRequest) async throws -> HTTPResponse
}

public struct HTTPResponse {
    public var status: Int
    /// The body's bytes in order, read as they are asked for, so nothing
    /// arrives faster than it is read. Once its iterator is let go the
    /// exchange is cancelled.
    public var body: any AsyncSequence<UInt8, any Error>

    public init(status: Int, body: any AsyncSequence<UInt8, any Error>) {
        self.status = status
        self.body = body
    }
}

/// The Thesys Gateway's Chat Completions endpoint, called directly with the
/// person's key (a recorded departure from Thesys's advice to keep the key on
/// a server; see ADR 0018). The request and the stream follow OpenAI's Chat
/// Completions format, which Thesys's docs say the Gateway implements.
public struct ThesysGateway: GatewayClient {
    public static let endpoint = URL(string: "https://api.thesys.dev/v1/embed/chat/completions")!
    /// A longer line from the Gateway is refused rather than buffered.
    /// This bounds the only bytes the app holds before the session sees them
    /// (one server-sent-event line). It equals the core's 1 MiB program-size
    /// limit (`MAX_PROGRAM_SIZE`), so no single event can carry more than the
    /// session would accept anyway. A safety bound, not a spend guard; the
    /// maintainer may change the number.
    public static let maxLineBytes = 1 << 20
    /// How much of an error response's body is read for its message.
    /// The rest is not read, so an endless error body cannot grow memory;
    /// 64 KiB holds any error message the app shows. A safety bound, not a
    /// spend guard; the maintainer may change the number.
    public static let maxErrorBodyBytes = 64 << 10
    static let missingKey = "Add your Thesys API key in Settings."
    /// What a shown message says in place of the key, should the Gateway
    /// ever echo it back.
    static let keyPlaceholder = "[your API key]"

    private let keyStore: any APIKeyStore
    private let transport: any HTTPTransport

    public init(keyStore: any APIKeyStore = KeychainKeyStore(), transport: any HTTPTransport = URLSessionTransport()) {
        self.keyStore = keyStore
        self.transport = transport
    }

    public func answer(_ request: ChatRequest) -> any AsyncSequence<String, any Error> {
        Answer(request: request, credential: { try Self.key(from: keyStore) }, transport: transport)
    }

    func answer(_ request: ChatRequest, credential: ConversationCredential) -> Answer {
        Answer(request: request, credential: { credential.key }, transport: transport)
    }

    /// One answer, read as it is asked for: nothing is read ahead of the
    /// reader, so how fast the app takes the text is how fast the network is
    /// read, and an answer the app stops taking holds nothing more.
    struct Answer: AsyncSequence, Sendable {
        let request: ChatRequest
        let credential: @Sendable () throws -> String
        let transport: any HTTPTransport

        func makeAsyncIterator() -> Iterator {
            Iterator(answer: self)
        }

        struct Iterator: AsyncIteratorProtocol {
            let answer: Answer
            /// The key lives only here, for the length of one request.
            private var key = ""
            private var body: (any AsyncIteratorProtocol<UInt8, any Error>)?
            private var lines = LineSplitter(limit: ThesysGateway.maxLineBytes)
            private var ended = false

            init(answer: Answer) { self.answer = answer }

            mutating func next() async throws -> String? {
                guard !ended else { return nil }
                do {
                    if let delta = try await read() { return delta }
                    ended = true
                    body = nil
                    try Task.checkCancellation()
                    return nil
                } catch {
                    ended = true
                    body = nil
                    throw ThesysGateway.shown(error, hiding: key)
                }
            }

            /// The next delta, or nil at the terminal marker or end of the body.
            private mutating func read() async throws -> String? {
                if body == nil {
                    key = try answer.credential()
                    let response = try await answer.transport.send(ThesysGateway.urlRequest(answer.request, key: key))
                    guard (200..<300).contains(response.status) else {
                        throw await ThesysGateway.refusal(status: response.status, body: response.body,
                                                          model: answer.request.model)
                    }
                    body = response.body.makeAsyncIterator()
                }
                while let line = try await nextLine() {
                    switch try ThesysGateway.event(fromLine: line, model: answer.request.model) {
                    case .delta(let text): return text
                    case .done: return nil
                    case .ignored: continue
                    }
                }
                return nil
            }

            private mutating func nextLine() async throws -> String? {
                while let byte = try await body?.next() {
                    if let line = try lines.push(byte) { return line }
                }
                return lines.finish()
            }
        }
    }

    static func key(from store: any APIKeyStore) throws -> String {
        let key: String?
        do { key = try store.read() } catch { throw GatewayError(error.description) }
        guard let key, !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw GatewayError(missingKey)
        }
        return key
    }

    /// The HTTP request: `POST`, the key as a bearer token, and a JSON body of
    /// the model, the messages in order and `stream: true`.
    static func urlRequest(_ request: ChatRequest, key: String) -> URLRequest {
        var urlRequest = URLRequest(url: endpoint)
        urlRequest.httpMethod = "POST"
        // The OpenAI SDK's default, which the template's server uses: a model
        // may think for a long while before its first token.
        urlRequest.timeoutInterval = 600
        urlRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
        urlRequest.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        let messages = request.messages.map { message in
            JSONValue.object([("role", .string(message.role.rawValue)), ("content", .string(message.content))])
        }
        let body = JSONValue.object([
            ("model", .string(request.model)),
            ("messages", .array(messages)),
            ("stream", .bool(true)),
        ])
        urlRequest.httpBody = Data(body.compact.utf8)
        return urlRequest
    }

    private enum Event {
        case delta(String)
        case done
        case ignored
    }

    /// `[DONE]` ends the reply even when the transport remains open. For
    /// deltas, the upstream adapter reads the first choice's `delta.content`
    /// from JSON on `data: ` lines. The OpenAI SDK reports JSON errors in-band.
    private static func event(fromLine line: String, model: String) throws(GatewayError) -> Event {
        guard line.hasPrefix("data:") else { return .ignored }
        let payload = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
        if payload == "[DONE]" { return .done }
        guard line.hasPrefix("data: "), let json = try? JSONValue.parse(payload) else { return .ignored }
        if let error = json["error"], error != .null {
            throw gatewayError(in: json, fallback: "The Gateway reported an error.", model: model)
        }
        guard let text = json["choices"]?.array?.first?["delta"]?["content"]?.string, !text.isEmpty else { return .ignored }
        return .delta(text)
    }

    /// The error for a refused request, with the Gateway's message read as
    /// upstream's `getResponseErrorMessage` reads it.
    static func refusal(status: Int, body: any AsyncSequence<UInt8, any Error>, model: String? = nil) async -> GatewayError {
        var bytes: [UInt8] = []
        do {
            var reader = body.makeAsyncIterator()
            while bytes.count < maxErrorBodyBytes, let byte = try await reader.next() {
                bytes.append(byte)
            }
        } catch {}
        let text = String(decoding: bytes, as: UTF8.self)
        return gatewayError(in: try? JSONValue.parse(text), fallback: "Request failed: \(status)", model: model, status: status)
    }

    private static func gatewayError(in json: JSONValue?, fallback: String, model: String?, status: Int? = nil) -> GatewayError {
        let message = json.flatMap(message(in:)) ?? fallback
        let nestedCode = json?["error"]?["code"]
        let code = nestedCode == .null ? json?["code"] : nestedCode ?? json?["code"]
        let possibleRefusal = status == nil || status == 400 || status == 403 || status == 404 || status == 422
        let modelRefused = possibleRefusal && refusesModel(model, code: code, message: message)
        return GatewayError(message, status: status, modelRefused: modelRefused)
    }

    private static func refusesModel(_ model: String?, code: JSONValue?, message: String) -> Bool {
        guard let model, !model.isEmpty else { return false }
        if let code, code != .null {
            guard code.string == "model_not_found" || code.string == "unsupported_model" else { return false }
            // A structured refusal can omit the ID, but a named provider/model
            // must be the requested one. Ignore sentence punctuation around it.
            let words = message.split { character in
                !character.isASCII || !(character.isLetter || character.isNumber || "._-/:".contains(character))
            }
            return words.allSatisfy { word in
                let parts = word.split(separator: "/", omittingEmptySubsequences: false)
                guard parts.count == 2, let provider = parts.first?.split(separator: ":").last,
                      provider.first?.isLetter == true || provider.first?.isNumber == true,
                      let name = parts.last, name.first?.isLetter == true || name.first?.isNumber == true
                else { return true }
                return "\(provider)/\(name.trimmingCharacters(in: CharacterSet(charactersIn: ".:")))" == model
            }
        }

        let message = message.trimmingCharacters(in: .whitespacesAndNewlines)
        let text = message.hasSuffix(".") ? message.dropLast() : message[...]
        let folded = text.lowercased()
        if folded == "unknown model" { return true }

        let namedModel: Substring
        if folded.hasPrefix("the model "), folded.hasSuffix(" does not exist") {
            namedModel = text.dropFirst("the model ".count).dropLast(" does not exist".count)
        } else if let prefix = ["unknown model:", "model not found:", "unsupported model:"]
            .first(where: { folded.hasPrefix($0) }) {
            namedModel = text.dropFirst(prefix.count)
        } else {
            return false
        }
        return [model, "\"\(model)\"", "'\(model)'", "`\(model)`"]
            .contains(namedModel.trimmingCharacters(in: .whitespaces))
    }

    /// `error` when it is text, else `error.message`, else `message`; nil when
    /// that is missing or blank.
    static func message(in json: JSONValue) -> String? {
        let message = json["error"]?.string ?? json["error"]?["message"]?.string ?? json["message"]?.string
        guard let message, !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        return message
    }

    /// What the person is shown for a failure: a cancellation stays one, and
    /// every message has the key taken out.
    static func shown(_ error: any Error, hiding key: String) -> any Error {
        if error is CancellationError || (error as? URLError)?.code == .cancelled || Task.isCancelled {
            return CancellationError()
        }
        let gatewayError = error as? GatewayError ?? GatewayError(error.localizedDescription)
        guard !key.isEmpty, gatewayError.message.contains(key) else { return gatewayError }
        return GatewayError(gatewayError.message.replacingOccurrences(of: key, with: keyPlaceholder),
                            status: gatewayError.status, modelRefused: gatewayError.modelRefused)
    }
}

/// Splits a byte stream into lines at `\n`, dropping a `\r` before it, and
/// refuses a line longer than its limit.
struct LineSplitter {
    let limit: Int
    private var line: [UInt8] = []

    init(limit: Int) { self.limit = limit }

    mutating func push(_ byte: UInt8) throws(GatewayError) -> String? {
        if byte == UInt8(ascii: "\n") { return take() }
        guard line.count < limit else {
            throw GatewayError("The Gateway sent a line longer than \(limit >> 20) MiB.")
        }
        line.append(byte)
        return nil
    }

    /// The last line, when the stream ends without a newline.
    mutating func finish() -> String? {
        line.isEmpty ? nil : take()
    }

    private mutating func take() -> String {
        if line.last == UInt8(ascii: "\r") { line.removeLast() }
        defer { line.removeAll(keepingCapacity: true) }
        return String(decoding: line, as: UTF8.self)
    }
}

/// HTTP over an ephemeral `URLSession`: no cache, cookies or credentials
/// are written to disk.
public struct URLSessionTransport: HTTPTransport {
    private let session: URLSession

    public init(session: URLSession = URLSession(configuration: .ephemeral)) {
        self.session = session
    }

    public func send(_ request: URLRequest) async throws -> HTTPResponse {
        let (bytes, response) = try await session.bytes(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        return HTTPResponse(status: status, body: CancellingBytes(bytes: bytes))
    }
}

/// URLSession's bytes, read straight through, so its own flow control holds
/// back the network while reading is slow; the data task is cancelled once
/// the iterator is let go, whether the body was read to its end or not.
struct CancellingBytes: AsyncSequence {
    typealias Element = UInt8
    let bytes: URLSession.AsyncBytes

    struct AsyncIterator: AsyncIteratorProtocol {
        var bytes: URLSession.AsyncBytes.AsyncIterator
        let cancel: TaskCanceller

        mutating func next() async throws -> UInt8? {
            try await bytes.next()
        }
    }

    func makeAsyncIterator() -> AsyncIterator {
        AsyncIterator(bytes: bytes.makeAsyncIterator(), cancel: TaskCanceller(bytes.task))
    }
}

/// Cancels a data task when it is let go; cancelling a finished task does
/// nothing.
final class TaskCanceller {
    private let task: URLSessionTask
    init(_ task: URLSessionTask) { self.task = task }
    deinit { task.cancel() }
}
