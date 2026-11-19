import Foundation
import OpenUINative
import Synchronization

/// Outcomes belong to a tool name, independently of its arguments. Each new
/// runtime gets its own cursors; the last outcome continues to serve refreshes.
struct ToolScript: Sendable {
    private struct Outcome: Sendable {
        let delay: Duration
        let result: Result<JSONValue, ToolProviderError>

        init(_ json: JSONValue, location: String) throws {
            guard case let .object(fields) = json else {
                throw DecodeError(description: "\(location) needs an outcome object")
            }
            let keys = fields.map(\.key)
            let kinds = keys.filter { $0 != "delayMs" }
            guard Set(keys).count == keys.count, kinds.count == 1, let kind = kinds.first,
                  let value = json[kind] else {
                throw DecodeError(description: "\(location) needs exactly one of value, error or failed, and optional delayMs")
            }
            switch (kind, value) {
            case ("value", _): result = .success(value)
            case let ("error", .string(text)): result = .failure(.toolReported(text))
            case let ("failed", .string(text)): result = .failure(.failed(text))
            default: throw DecodeError(description: "\(location) needs value (JSON), error (text) or failed (text)")
            }
            if let delay = json["delayMs"] {
                guard let n = delay.number, let ms = Int32(exactly: n), ms >= 0 else {
                    throw DecodeError(description: "\(location).delayMs must be a whole number from 0 to 2147483647")
                }
                self.delay = .milliseconds(ms)
            } else {
                delay = .zero
            }
        }
    }

    private let outcomes: [String: [Outcome]]
    private let names: [String]

    init(contentsOf url: URL) throws {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        try self.init(json: String(contentsOf: url, encoding: .utf8))
    }

    init(json: String) throws {
        guard case let .object(tools) = try JSONValue.parse(json) else {
            throw DecodeError(description: "A tool script needs an object keyed by tool name")
        }
        var outcomes: [String: [Outcome]] = [:]
        for (name, value) in tools {
            guard outcomes[name] == nil, let steps = value.array, !steps.isEmpty else {
                throw DecodeError(description: "\(name) needs one nonempty array of outcomes")
            }
            outcomes[name] = try steps.enumerated().map { index, step in
                try Outcome(step, location: "\(name)[\(index)]")
            }
        }
        self.outcomes = outcomes
        names = tools.map(\.key)
    }

    func provider<C: Clock>(clock: C) -> ToolProvider where C.Duration == Duration {
        let indices = Mutex<[String: Int]>([:])
        return { name, _ in
            guard let steps = outcomes[name] else {
                throw ToolProviderError.toolNotFound(availableTools: names)
            }
            // Reserve before suspending: overlapping calls take separate steps.
            let step = indices.withLock { indices in
                let index = indices[name, default: 0]
                indices[name] = min(index + 1, steps.count - 1)
                return steps[index]
            }
            if step.delay > .zero { try await clock.sleep(for: step.delay) }
            return try step.result.get()
        }
    }
}
