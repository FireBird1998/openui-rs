import Foundation
import OpenUIFFI

/// Performs one tool call for a live program's queries and mutations: the
/// evaluated tool name and arguments in, the tool's result out. `arguments`
/// is nil when the program gave none (undefined). Throw `ToolProviderError`
/// to choose the failure; any other error is a failed call with the error's
/// description. The closure runs off the main actor, once per call, and many
/// calls can be in flight at once. Nothing times it out: a call that never
/// returns leaves its query loading while its registration stays active. An
/// app that wants a timeout applies it here (ADR 0013).
public typealias ToolProvider = @Sendable (_ toolName: String, _ arguments: JSONValue?) async throws -> JSONValue

/// The three ways a tool call fails, as upstream's tool provider reports
/// them. Each becomes the query's (or mutation's) error with upstream's code.
public enum ToolProviderError: Error, Sendable, Equatable {
    /// No tool has this name. Code `tool-not-found`; the available names,
    /// in order, become the error's hint.
    case toolNotFound(availableTools: [String])
    /// The tool ran and reported an error, as an MCP tool's `isError`
    /// response does. Code `mcp-error`.
    case toolReported(String)
    /// The call itself failed. Code `tool-error`; this is also what any
    /// other thrown error becomes.
    case failed(String)
}

/// What a live program's queries are doing, from its last evaluation with a
/// tool provider attached. Empty without one.
public struct QueryStatus: Sendable, Equatable {
    /// Statement values from the query snapshot, including defaults and mutation status.
    public var values: JSONValue = .object([])
    /// Statement ids of the queries waiting for a result, in upstream's order.
    public var loading: [String] = []
    /// Of those, the ones that have shown a fetched result before.
    public var refetching: [String] = []
    /// The queries' and mutations' errors, in upstream's order.
    public var errors: [QueryIssue] = []
    /// The evaluation diagnostic, including readers still changing at the round limit.
    public var evaluationError: String?

    public init() {}

    init(snapshot: String?, evaluationError: OpenUiError?) {
        self.evaluationError = evaluationError.map(LiveProgram.describe)
        guard let snapshot, let json = try? JSONValue.parse(snapshot),
              case let .object(fields) = json else { return }
        let metadata = ["__openui_loading", "__openui_refetching", "__openui_errors"]
        values = .object(fields.filter { !metadata.contains($0.key) })
        loading = (json["__openui_loading"]?.array ?? []).compactMap(\.string)
        refetching = (json["__openui_refetching"]?.array ?? []).compactMap(\.string)
        errors = (json["__openui_errors"]?.array ?? []).map(QueryIssue.init(json:))
    }
}

/// A query's or mutation's error, with upstream's fields.
public struct QueryIssue: Sendable, Equatable {
    /// `query` or `mutation`.
    public var source: String
    /// `tool-not-found`, `mcp-error` or `tool-error`.
    public var code: String
    public var message: String
    public var statementId: String?
    /// The tool name as the program gave it (not always a string).
    public var toolName: JSONValue?
    public var hint: String?
    /// The error as the runtime gave it.
    public var json: JSONValue

    init(json: JSONValue) {
        source = json["source"]?.string ?? ""
        code = json["code"]?.string ?? ""
        message = json["message"]?.string ?? ""
        statementId = json["statementId"]?.string
        toolName = json["toolName"]
        hint = json["hint"]?.string
        self.json = json
    }
}

/// The thin loop between a runtime session's query manager and an app's
/// tool provider. It holds no scheduling rules: it performs the calls the
/// session wants, reports each outcome, arms one timer for the deadline the
/// session reports and tells the session the time when it fires. Everything
/// runs on the main actor, as the live program's other session calls do,
/// except the provider itself.
///
/// The live program owns the driver; the driver's tasks hold it weakly, so
/// neither a call in flight nor the timer keeps the live program alive. The
/// driver cancels its tasks when released, including off the main actor.
@MainActor final class QueryDriver {
    private weak var program: LiveProgram?
    private let session: RuntimeSession
    private let provider: ToolProvider
    /// Milliseconds since attaching, on the app's monotonic clock.
    private let elapsed: @Sendable () -> UInt64
    /// Sleeps until that many milliseconds since attaching.
    private let sleep: @Sendable (UInt64) async throws -> Void
    private(set) var calls: [ObjectIdentifier: Task<Void, Never>] = [:]
    private(set) var timer: (deadline: UInt64, task: Task<Void, Never>)?
    private var lastTime: UInt64 = 0
    private(set) var paused = false

    deinit {
        for call in calls.values { call.cancel() }
        timer?.task.cancel()
    }

    init<C: Clock>(program: LiveProgram, session: RuntimeSession, clock: C, provider: @escaping ToolProvider)
    where C.Duration == Duration {
        self.program = program
        self.session = session
        self.provider = provider
        let origin = clock.now
        elapsed = { Self.milliseconds(origin.duration(to: clock.now)) }
        sleep = { ms in try await clock.sleep(until: origin.advanced(by: .milliseconds(ms)), tolerance: nil) }
    }

    func now() -> UInt64 { max(lastTime, elapsed()) }

    /// A draw, write or completion can install a refresh interval between timer wakes.
    func advanceClock() throws {
        let time = now()
        guard time > lastTime else { return }
        try session.advanceQueryClock(nowMs: time)
        lastTime = time
    }

    func dispatch(_ calls: [WantedToolCall], deadline: UInt64?) {
        for call in calls { start(call) }
        arm(deadline)
    }

    func pause() throws {
        try session.pauseQueries()
        paused = true
        stop()
    }

    func resume() throws {
        let time = now()
        try session.resumeQueriesAt(nowMs: time)
        lastTime = time
        paused = false
    }

    func stop() {
        for call in calls.values { call.cancel() }
        calls.removeAll()
        timer?.task.cancel()
        timer = nil
    }

    private func start(_ call: WantedToolCall) {
        let id = ObjectIdentifier(call)
        let name = call.toolName()
        let arguments = call.argumentsJson()
        calls[id] = Task { [weak self, provider] in
            guard !Task.isCancelled else { return }
            let outcome = await Self.perform(provider, name: name, arguments: arguments)
            // A provider can cancel its own task. Only a call we retired on
            // pause or detach disappears from the set without an outcome.
            guard let self, calls.removeValue(forKey: id) != nil else { return }
            complete(call, Task.isCancelled ? Self.failed("cancelled by provider") : outcome)
        }
    }

    private func complete(_ call: WantedToolCall, _ outcome: String) {
        guard let program else { return }
        // The clock moves first, as before, but its failure must not skip
        // the delivery: a receipt that is never finished stays live.
        do { try advanceClock() } catch { program.queryFailure(error) }
        do {
            // This is the provider's final answer. The session settles an
            // outcome it refuses (over its limits or invalid JSON, an
            // oversized failure message included) as the call's tool error,
            // so the query shows an error rather than loading for ever.
            try session.finishToolCall(call: call, outcomeJson: outcome)
        } catch {
            program.queryFailure(error)
        }
        program.syncQueries()
    }

    private func arm(_ deadline: UInt64?) {
        guard deadline != timer?.deadline else { return }
        timer?.task.cancel()
        timer = nil
        guard let deadline else { return }
        timer = (deadline, Task { [weak self, sleep] in
            do {
                try await sleep(deadline)
                guard !Task.isCancelled, let self, let program else { return }
                timer = nil
                try advanceClock()
                program.syncQueries()
            } catch {
                guard !Task.isCancelled, let self else { return }
                timer = nil
                program?.queryFailure(error)
            }
        })
    }

    /// Runs the provider and turns what it returns or throws into the
    /// session's outcome JSON. Off the main actor.
    nonisolated private static func perform(_ provider: ToolProvider, name: String, arguments: String?) async -> String {
        let parsed: JSONValue?
        do { parsed = try arguments.map { try JSONValue.parse($0) } } catch { return failed(String(describing: error)) }
        do {
            try Task.checkCancellation()
            let value = try await provider(name, parsed)
            return JSONValue.object([("value", value)]).compact
        } catch let error as ToolProviderError {
            switch error {
            case let .toolNotFound(names):
                return JSONValue.object([("availableTools", .array(names.map(JSONValue.string)))]).compact
            case let .toolReported(text):
                return JSONValue.object([("error", .string(text))]).compact
            case let .failed(text):
                return failed(text)
            }
        } catch is CancellationError {
            return failed("cancelled by provider")
        } catch {
            return failed((error as? LocalizedError)?.errorDescription ?? String(describing: error))
        }
    }

    nonisolated private static func failed(_ message: String) -> String {
        JSONValue.object([("failed", .string(message))]).compact
    }

    nonisolated static func milliseconds(_ duration: Duration) -> UInt64 {
        let (seconds, attoseconds) = duration.components
        guard seconds >= 0 else { return 0 }
        let (ms, overflow) = UInt64(seconds).multipliedReportingOverflow(by: 1000)
        guard !overflow else { return UInt64.max }
        let (total, overflowed) = ms.addingReportingOverflow(UInt64(max(0, attoseconds) / 1_000_000_000_000_000))
        return overflowed ? UInt64.max : total
    }
}
