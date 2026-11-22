import OpenUIFFI

// The message binding admits the JSON of the parts under MAX_PROGRAM_SIZE.
private let messageByteLimit = 1024 * 1024
private let messageTooLarge = OpenUIError(.limitExceeded, "message exceeds the program-size limit")

/// The winning content section, its marker attributes, and any following context or end marker.
public struct ParsedMessage: Sendable {
    public let content: String
    public let context: JSONValue
    public let attributes: [(key: String, value: String)]
    public let end: Bool
}

/// The two outcomes of an MCP tool that answered. An unreadable response throws OpenUIError.
public enum McpToolOutcome: Sendable, Equatable {
    case value(JSONValue)
    case toolReported(String)
}

extension Core {
    /// Writes a message with lang-core's marker lines. Attribute pairs preserve
    /// insertion order. A string context is written verbatim and null is omitted.
    public func buildMessage(content: String, context: JSONValue? = nil,
                             attributes: [(key: String, value: String)] = [],
                             end: Bool = false) async throws(OpenUIError) -> String {
        try await run {
            // Each attribute needs at least five bytes in the parts JSON ("":"").
            guard attributes.count <= messageByteLimit / 5 else { throw messageTooLarge }
            var parts: [(key: String, value: JSONValue)] = [("content", .string(content))]
            if let context { parts.append(("context", context)) }
            if !attributes.isEmpty {
                parts.append(("attributes", .object(attributes.map { ($0.key, .string($0.value)) })))
            }
            if end { parts.append(("end", .bool(true))) }
            guard let json = JSONValue.object(parts).compact(byteLimit: messageByteLimit) else { throw messageTooLarge }
            return try OpenUIFFI.buildMessage(parts: json)
        }
    }

    /// Reads a message. While streaming, a trailing partial marker is held back.
    public func parseMessage(_ text: String, streaming: Bool = false) async throws(OpenUIError) -> ParsedMessage {
        try await run {
            guard text.utf8.count <= messageByteLimit else { throw messageTooLarge }
            let parts = try JSONValue.parse(OpenUIFFI.parseMessage(text: text, streaming: streaming))
            guard let content = parts["content"]?.string, let context = parts["context"],
                  case let .object(entries)? = parts["attributes"], let end = parts["end"]?.bool else {
                throw OpenUIError(.decode, "Missing parsed message fields")
            }
            let attributes = try entries.map { key, field in
                guard let value = field.string else { throw OpenUIError(.decode, "Invalid message attribute") }
                return (key, value)
            }
            return ParsedMessage(content: content, context: context, attributes: attributes, end: end)
        }
    }

    /// Reads an MCP callTool response as received on the wire, using the core's default limits.
    public func extractMcpToolResult(_ responseJSON: String) async throws(OpenUIError) -> McpToolOutcome {
        try await run {
            // Refuse before UniFFI copies a string larger than the default result ceiling.
            guard responseJSON.utf8.count <= 8 * 1024 * 1024 else {
                throw OpenUIError(.limitExceeded, "work or size limit exceeded")
            }
            let outcome = try JSONValue.parse(OpenUIFFI.extractMcpToolResult(responseJson: responseJSON, limits: ParseLimits()))
            if let value = outcome["value"] { return .value(value) }
            if let text = outcome["error"]?.string { return .toolReported(text) }
            throw OpenUIError(.decode, "Missing MCP tool outcome")
        }
    }

    /// Builds the stored user turn for a click or form submit. A supplied
    /// action context (including JSON null) occupies the slot after form state.
    public func buildActionUserMessage(label: String, formState: JSONValue? = nil,
                                       context: JSONValue? = nil) async throws(OpenUIError) -> String {
        guard label.utf8.count <= messageByteLimit else { throw messageTooLarge }
        var parts: [JSONValue] = [.string("User clicked: \(label)")]
        if let context {
            if let formState, formState != .null { parts.append(formState) }
            else { parts.append(.object([])) }
            parts.append(context)
        } else if let formState {
            // The upstream presence check uses JavaScript truthiness. An empty form is truthy.
            switch formState {
            case .null, .bool(false), .string(""): break
            case let .number(n) where n == 0 || n.isNaN: break
            default: parts.append(formState)
            }
        }
        let built = try await buildMessage(content: label, context: .array(parts))
        // Upstream drops the first marker line when there is no label.
        return label.isEmpty ? String(built.drop(while: { $0 != "\n" }).dropFirst()) : built
    }

    /// Builds the next user message from a continue_conversation host event.
    public func buildActionUserMessage(_ event: HostEvent) async throws(OpenUIError) -> String {
        try await buildActionUserMessage(label: event.humanFriendlyMessage, formState: event.formState,
                                         context: event.params["context"])
    }
}
