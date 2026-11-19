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
    /// replace the parser's input even when the raw response only grew.
    public func response(_ text: String, streaming: Bool) async throws -> Step {
        try await core.run { [inner] in
            let bundle = try JSONValue.parse(parseResponseBundle(text: text, streaming: streaming))
            guard let program = bundle["program"]?.string, let complete = bundle["complete"]?.bool,
                  let isBundle = bundle["isBundle"]?.bool, let scripts = bundle["scripts"]?.array else {
                throw DecodeError(description: "Missing response bundle fields")
            }
            let state = ResponseState(streaming: streaming, isBundle: isBundle, complete: complete,
                error: bundle["error"]?.string, scripts: scripts.compactMap(\.string),
                scriptRevision: bundle["scriptRevision"]?.string)
            // The bundle reader returns the parser input. Only the message
            // reader exposes context and attributes (even for context alone).
            var context = JSONValue.null
            var attributes: [String: String] = [:]
            if text.contains("]]>") {
                let message = try JSONValue.parse(parseMessage(text: text, streaming: streaming))
                context = message["context"] ?? .null
                if case let .object(entries)? = message["attributes"] {
                    for (key, value) in entries { attributes[key] = value.string }
                }
            }
            var step = try Step.measure(chunks: 1, streaming: streaming) {
                let partial = inner.set(text: program)
                return streaming ? partial : inner.finish()
            }
            if isBundle || program != text { step.program = program }
            step.response = ResponseMetadata(name: bundle["metadata"]?["name"]?.string, context: context,
                attributes: attributes, sourceByteCount: text.utf8.count, state: state)
            return step
        }
    }
}
