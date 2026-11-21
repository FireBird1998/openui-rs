import Foundation
import OpenUIFFI

/// Metadata for the winning content section of the response on screen.
public struct ResponseMetadata: Sendable {
    public let name: String?
    public let context: JSONValue
    public let attributes: [String: String]
    /// Size of the source used to estimate retention when caching parsed steps.
    public let sourceByteCount: Int
    let state: ResponseState

    public var error: String? { state.error }
}

extension Session {
    /// Reads the whole response so far. A regenerated message or an edit can
    /// replace the parser's input even when the raw response only grew. The
    /// reader remembers how far it got, so a response that only grows costs
    /// about what pushing it does; any other edit is read afresh.
    /// Refuses what the core's response reader refuses, with its kind
    /// (`.limitExceeded`, `.invalidJSON`, `.invalidAttribute`, …).
    public func response(_ text: String, streaming: Bool) async throws(OpenUIError) -> Step {
        try await core.run { [inner] in
            let started = ContinuousClock.now
            // The session's own parser reads it, so `finish`, `push` and
            // `set` go on from this program as before.
            let read = try inner.readResponse(text: text, streaming: streaming)
            let called = ContinuousClock.now
            let bundle = try JSONValue.parse(read.bundle)
            guard let program = bundle["program"]?.string, let complete = bundle["complete"]?.bool,
                  let isBundle = bundle["isBundle"]?.bool, let scripts = bundle["scripts"]?.array else {
                throw OpenUIError(.decode, "Missing response bundle fields")
            }
            let state = ResponseState(streaming: streaming, isBundle: isBundle, complete: complete,
                error: bundle["error"]?.string, scripts: scripts.compactMap(\.string),
                scriptRevision: bundle["scriptRevision"]?.string)
            // The bundle reader returns the parser input. Only the message
            // reader exposes context and attributes (even for context alone);
            // the reader gives them when the text has a marker.
            var context = JSONValue.null
            var attributes: [String: String] = [:]
            if let messageJSON = read.message {
                let message = try JSONValue.parse(messageJSON)
                context = message["context"] ?? .null
                if case let .object(entries)? = message["attributes"] {
                    for (key, value) in entries { attributes[key] = value.string }
                }
            }
            // The read parsed the program already. Its parse time covers the
            // core's reading and parsing; the rest of the call is crossing.
            var step = try Step.measure(chunks: 1, streaming: streaming) { read.parse }
            step.timings.crossing = (called - started) - step.timings.parse
            if isBundle || program != text { step.program = program }
            step.response = ResponseMetadata(name: bundle["metadata"]?["name"]?.string, context: context,
                attributes: attributes, sourceByteCount: text.utf8.count, state: state)
            return step
        }
    }
}
