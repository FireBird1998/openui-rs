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

    public init(_ message: String, status: Int? = nil) {
        self.message = message
        self.status = status
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
        Answer(request: request, keyStore: keyStore, transport: transport)
    }

    /// One answer, read as it is asked for: nothing is read ahead of the
    /// reader, so how fast the app takes the text is how fast the network is
    /// read, and an answer the app stops taking holds nothing more.
    struct Answer: AsyncSequence {
        let request: ChatRequest
        let keyStore: any APIKeyStore
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

            /// The next delta, or nil at the end of the body.
            private mutating func read() async throws -> String? {
                if body == nil {
                    key = try ThesysGateway.key(from: answer.keyStore)
                    let response = try await answer.transport.send(ThesysGateway.urlRequest(answer.request, key: key))
                    guard (200..<300).contains(response.status) else {
                        throw await ThesysGateway.refusal(status: response.status, body: response.body)
                    }
                    body = response.body.makeAsyncIterator()
                }
                while let byte = try await body?.next() {
                    if let line = try lines.push(byte), let delta = try ThesysGateway.delta(fromLine: line) {
                        return delta
                    }
                }
                if let line = lines.finish(), let delta = try ThesysGateway.delta(fromLine: line) {
                    return delta
                }
                return nil
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

    /// The text one server-sent event line adds to the answer, if any. As
    /// upstream's OpenAI adapter does, only `data: ` lines count, `[DONE]` and
    /// lines that are not JSON are skipped, and the text is the first
    /// choice's `delta.content`. A line whose JSON has an `error` ends the
    /// answer with its message, as the OpenAI SDK reads one.
    static func delta(fromLine line: String) throws(GatewayError) -> String? {
        guard line.hasPrefix("data: ") else { return nil }
        let payload = line.dropFirst(6).trimmingCharacters(in: .whitespaces)
        guard !payload.isEmpty, payload != "[DONE]", let json = try? JSONValue.parse(payload) else { return nil }
        if let error = json["error"], error != .null {
            throw GatewayError(message(in: .object([("error", error)])) ?? "The Gateway reported an error.")
        }
        guard let text = json["choices"]?.array?.first?["delta"]?["content"]?.string, !text.isEmpty else { return nil }
        return text
    }

    /// The error for a refused request, with the Gateway's message read as
    /// upstream's `getResponseErrorMessage` reads it.
    static func refusal(status: Int, body: any AsyncSequence<UInt8, any Error>) async -> GatewayError {
        var bytes: [UInt8] = []
        do {
            for try await byte in body {
                guard bytes.count < maxErrorBodyBytes else { break }
                bytes.append(byte)
            }
        } catch {}
        let text = String(decoding: bytes, as: UTF8.self)
        let message = (try? JSONValue.parse(text)).flatMap(message(in:))
        return GatewayError(message ?? "Request failed: \(status)", status: status)
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
                            status: gatewayError.status)
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
