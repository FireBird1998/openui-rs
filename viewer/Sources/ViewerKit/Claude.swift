import Foundation
import Observation

/// A Claude model the viewer can ask, with its price in dollars per million
/// tokens (Claude API reference, checked 2026-10-15; thinking bills as output).
public struct ClaudeModel: Identifiable, Hashable, Sendable {
    public var id: String
    public var name: String
    public var inputPrice: Double
    public var outputPrice: Double
    /// Whether the API can retry a declined request on another model
    /// (`fallbacks: "default"`); Claude Haiku 5.5 has no fallback.
    public var fallbacks: Bool

    public static let opus = ClaudeModel(id: "claude-opus-5-5", name: "Claude Opus 5.5", inputPrice: 4, outputPrice: 20, fallbacks: true)
    public static let sonnet = ClaudeModel(id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", inputPrice: 2, outputPrice: 10, fallbacks: true)
    /// Prices for prompts up to 100K tokens; the viewer's prompts are far smaller.
    public static let haiku = ClaudeModel(id: "claude-haiku-5-5", name: "Claude Haiku 5.5", inputPrice: 0.10, outputPrice: 0.50, fallbacks: false)
    public static let all = [opus, sonnet, haiku]

    /// The estimated cost of a request, in dollars. Cached prompt reads cost
    /// less than this; it errs high.
    public func cost(inputTokens: Int, outputTokens: Int) -> Double {
        (Double(inputTokens) * inputPrice + Double(outputTokens) * outputPrice) / 1_000_000
    }
}

/// What the viewer asks Claude for.
public struct ClaudeRequest: Sendable {
    public var model: ClaudeModel
    /// `low`, `medium` or `high`.
    public var effort: String
    public var systemPrompt: String
    public var userText: String
    /// Bounds a single request's cost: 16,000 tokens is at most $0.32 on Opus.
    public var maxTokens = 16_000

    public init(model: ClaudeModel, effort: String, systemPrompt: String, userText: String) {
        self.model = model
        self.effort = effort
        self.systemPrompt = systemPrompt
        self.userText = userText
    }

    /// A follow-up's message: the current program, then the change wanted.
    /// The edit-mode system prompt asks for only the statements that change.
    public static func followUp(_ change: String, to program: String) -> String {
        "Here is the current program:\n\n```\n\(program)\n```\n\n\(change)"
    }

    /// The HTTP request for `POST /v1/messages`, streaming. The key goes only
    /// into the `x-api-key` header, never into anything logged or saved.
    func urlRequest(apiKey: String) throws -> URLRequest {
        var request = URLRequest(url: URL(string: "https://api.anthropic.com/v1/messages")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.setValue("2023-06-01", forHTTPHeaderField: "anthropic-version")
        request.setValue(apiKey, forHTTPHeaderField: "x-api-key")
        var body: [String: Any] = [
            "model": model.id,
            "max_tokens": maxTokens,
            "stream": true,
            "system": systemPrompt,
            // The system prompt is large and the same every time; cache it.
            "cache_control": ["type": "ephemeral"],
            "output_config": ["effort": effort],
            "messages": [["role": "user", "content": userText]],
        ]
        if model.fallbacks {
            request.setValue("server-side-fallback-2026-07-01", forHTTPHeaderField: "anthropic-beta")
            body["fallbacks"] = "default"
        }
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        return request
    }
}

/// One event from the Messages API's server-sent event stream, as the viewer
/// uses it.
public enum ClaudeEvent: Equatable, Sendable {
    /// The served model and the prompt's tokens.
    case started(model: String, inputTokens: Int)
    case thinking
    case text(String)
    /// The request was declined by one model and continued on another.
    case fallback(from: String, to: String)
    /// Output tokens so far, and why generation stopped once it has.
    case progress(outputTokens: Int, stopReason: String?)
    case failed(String)
}

/// Turns server-sent event lines into `ClaudeEvent`s. Each `data:` line holds
/// one JSON event whose `type` says what it is; `event:` lines repeat that and
/// are skipped.
public struct ServerSentEvents {
    public init() {}

    public func events(fromLine line: String) -> [ClaudeEvent] {
        guard line.hasPrefix("data:") else { return [] }
        let payload = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
        guard let json = try? JSONValue.parse(payload), let type = json["type"]?.string else { return [] }
        switch type {
        case "message_start":
            let message = json["message"]
            return [.started(
                model: message?["model"]?.string ?? "",
                inputTokens: Self.inputTokens(message?["usage"])
            )]
        case "content_block_start":
            let block = json["content_block"]
            switch block?["type"]?.string {
            case "thinking", "redacted_thinking": return [.thinking]
            case "fallback":
                return [.fallback(from: block?["from"]?["model"]?.string ?? "", to: block?["to"]?["model"]?.string ?? "")]
            case "text":
                let text = block?["text"]?.string ?? ""
                return text.isEmpty ? [] : [.text(text)]
            default: return []
            }
        case "content_block_delta":
            guard json["delta"]?["type"]?.string == "text_delta", let text = json["delta"]?["text"]?.string else { return [] }
            return text.isEmpty ? [] : [.text(text)]
        case "message_delta":
            return [.progress(
                outputTokens: Int(json["usage"]?["output_tokens"]?.number ?? 0),
                stopReason: json["delta"]?["stop_reason"]?.string
            )]
        case "error":
            return [.failed(json["error"]?["message"]?.string ?? "the API reported an error")]
        default:
            return []
        }
    }

    /// Input tokens, counting cache writes and reads too.
    static func inputTokens(_ usage: JSONValue?) -> Int {
        ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
            .reduce(0) { $0 + Int(usage?[$1]?.number ?? 0) }
    }
}

public struct ClaudeError: Error, CustomStringConvertible {
    public let description: String
}

/// Where a live request is, for the UI.
@MainActor @Observable
public final class LiveStatus {
    public enum Phase: Equatable {
        case connecting, thinking, writing
        case finished(stopReason: String)
        case failed(String)
        case cancelled
    }

    public let model: ClaudeModel
    public private(set) var phase = Phase.connecting
    public private(set) var servedBy: String?
    public private(set) var fallback: String?
    public private(set) var inputTokens = 0
    public private(set) var outputTokens = 0
    /// Characters of text received, for estimating output when a request is cancelled.
    public private(set) var receivedCharacters = 0
    /// Every text delta as it arrived, with seconds since the request was sent.
    public private(set) var received: [Recording.Chunk] = []

    public init(model: ClaudeModel) {
        self.model = model
    }

    public var isActive: Bool {
        switch phase {
        case .connecting, .thinking, .writing: true
        default: false
        }
    }

    /// The request's estimated cost so far. Before the API reports output
    /// tokens (it does so at the end), output is guessed from the text so far,
    /// which misses thinking, so the guess runs low.
    public var cost: Double {
        let output = outputTokens > 0 ? outputTokens : receivedCharacters / 3
        return model.cost(inputTokens: inputTokens, outputTokens: output)
    }

    func apply(_ event: ClaudeEvent) {
        switch event {
        case let .started(model, inputTokens):
            servedBy = model
            self.inputTokens = inputTokens
        case .thinking:
            if phase == .connecting { phase = .thinking }
        case let .text(text):
            receivedCharacters += text.count
            phase = .writing
        case let .fallback(from, to):
            fallback = "\(from) declined; \(to) continued"
        case let .progress(outputTokens, stopReason):
            self.outputTokens = outputTokens
            if let stopReason { phase = .finished(stopReason: stopReason) }
        case let .failed(message):
            phase = .failed(message)
        }
    }

    func receive(_ text: String, after elapsed: Duration) {
        received.append(Recording.Chunk(t: (elapsed.inSeconds * 1000).rounded() / 1000, text: text))
    }

    func end(_ error: (any Error)?) {
        guard isActive else { return }
        if error is CancellationError || (error as? URLError)?.code == .cancelled {
            phase = .cancelled
        } else if let error {
            phase = .failed(String(describing: error))
        } else {
            phase = .finished(stopReason: "end of stream")
        }
    }
}

/// Chunks from a live Claude stream: each text delta, as it arrives, is one
/// chunk. Cancelling the consumer cancels the HTTP request.
public struct ClaudeSource: ChunkSource {
    let request: ClaudeRequest
    let apiKey: String
    let status: LiveStatus
    let session: URLSession

    public init(request: ClaudeRequest, apiKey: String, status: LiveStatus, session: URLSession = .shared) {
        self.request = request
        self.apiKey = apiKey
        self.status = status
        self.session = session
    }

    public func chunks() -> AsyncThrowingStream<[String], any Error> {
        let (stream, continuation) = AsyncThrowingStream<[String], any Error>.makeStream()
        let task = Task { [request, apiKey, status, session] in
            let sent = ContinuousClock.now
            do {
                let (bytes, response) = try await session.bytes(for: request.urlRequest(apiKey: apiKey))
                if let http = response as? HTTPURLResponse, http.statusCode != 200 {
                    throw ClaudeError(description: try await Self.errorMessage(from: bytes, status: http.statusCode))
                }
                let parser = ServerSentEvents()
                for try await line in bytes.lines {
                    for event in parser.events(fromLine: line) {
                        await status.apply(event)
                        switch event {
                        case let .text(text):
                            await status.receive(text, after: ContinuousClock.now - sent)
                            continuation.yield([text])
                        case let .failed(message): throw ClaudeError(description: message)
                        default: break
                        }
                    }
                }
                await status.end(nil)
                continuation.finish()
            } catch {
                await status.end(error)
                continuation.finish(throwing: error)
            }
        }
        continuation.onTermination = { _ in task.cancel() }
        return stream
    }

    /// The API's error message from a non-200 response body.
    static func errorMessage(from bytes: URLSession.AsyncBytes, status: Int) async throws -> String {
        var body = Data()
        for try await byte in bytes {
            body.append(byte)
            if body.count > 64 << 10 { break }
        }
        let text = String(decoding: body, as: UTF8.self)
        let message = (try? JSONValue.parse(text))?["error"]?["message"]?.string
        return "HTTP \(status): \(message ?? text.prefix(300).description)"
    }
}
