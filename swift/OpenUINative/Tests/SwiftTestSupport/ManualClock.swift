import Synchronization

/// A monotonic clock that only moves when the test advances it.
public final class ManualClock: Clock, Sendable {
    public struct Instant: InstantProtocol {
        public var offset: Duration
        public func advanced(by duration: Duration) -> Instant { Instant(offset: offset + duration) }
        public func duration(to other: Instant) -> Duration { other.offset - offset }
        public static func < (a: Instant, b: Instant) -> Bool { a.offset < b.offset }
    }

    private struct Sleeper {
        let deadline: Instant
        let continuation: CheckedContinuation<Void, any Error>
    }

    private struct State {
        var now = Instant(offset: .zero)
        var next = 0
        var sleepers: [Int: Sleeper] = [:]
        var requested: [Instant] = []
    }

    private let state = Mutex(State())

    public init() {}

    public var now: Instant { state.withLock { $0.now } }
    public var minimumResolution: Duration { .milliseconds(1) }

    public func sleep(until deadline: Instant, tolerance: Duration?) async throws {
        let id = state.withLock { $0.next += 1; return $0.next }
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
                let immediate: Result<Void, any Error>? = state.withLock { state in
                    state.requested.append(deadline)
                    if Task.isCancelled { return .failure(CancellationError()) }
                    if deadline <= state.now { return .success(()) }
                    state.sleepers[id] = Sleeper(deadline: deadline, continuation: continuation)
                    return nil
                }
                if let immediate { continuation.resume(with: immediate) }
            }
        } onCancel: {
            let sleeper = state.withLock { state in
                state.sleepers.removeValue(forKey: id)
            }
            sleeper?.continuation.resume(throwing: CancellationError())
        }
    }

    /// The deadlines someone is sleeping until, in milliseconds since zero.
    public var deadlines: [Int64] {
        state.withLock { $0.sleepers.values.map { Int64($0.deadline.offset / .milliseconds(1)) }.sorted() }
    }

    public var requestedDeadlines: [Int64] {
        state.withLock { $0.requested.map { Int64($0.offset / .milliseconds(1)) } }
    }

    /// Moves the clock to `ms` milliseconds and wakes every sleeper now due.
    public func advance(to ms: Int64) {
        let due = state.withLock { state in
            state.now = Instant(offset: .milliseconds(ms))
            let due = state.sleepers.filter { $0.value.deadline <= state.now }
            for id in due.keys { state.sleepers[id] = nil }
            return due.values.map(\.continuation)
        }
        for continuation in due { continuation.resume() }
    }

    /// Moves the clock to `ms` milliseconds without waking anyone, like a
    /// timer that is due but hasn't had its turn yet.
    public func move(to ms: Int64) {
        state.withLock { state in
            let to = Instant(offset: .milliseconds(ms))
            precondition(state.now <= to, "a monotonic clock can't move backwards")
            state.now = to
        }
    }
}
