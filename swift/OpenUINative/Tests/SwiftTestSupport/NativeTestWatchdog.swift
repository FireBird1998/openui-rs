import Darwin
import Dispatch
import Synchronization
import Testing

/// A fixed wall-clock deadline for each test case. It can stop AppKit calls
/// and continuations that do not respond to Swift task cancellation.
///
/// The limit means "60 s from the body's first line", not 60 s from when
/// Swift Testing starts the case. Most suites are `@MainActor`, and their
/// bodies wait for the main actor behind other tests' work, so the clock
/// starts in a main-actor job queued just before the body's own first
/// main-actor job (see `run`). Once started it never pauses: when the body
/// suspends and later waits for the main actor again, that wait counts.
public struct NativeTestWatchdog: SuiteTrait, TestTrait, TestScoping {
    public let isRecursive = true

    public init() {}

    private struct State {
        var operations = ["the test body"]
        var expired = false
    }

    private final class Wait: Sendable {
        let state = Mutex(State())

        func finish(_ watchdog: DispatchWorkItem) {
            let expired = state.withLock {
                $0.operations.removeAll()
                return $0.expired
            }
            watchdog.cancel()
            // Once the timer wins, keep the runner alive for capture and
            // prevent a late body completion from reporting a passing case.
            if expired { DispatchSemaphore(value: 0).wait() }
        }
    }

    @TaskLocal private static var wait: Wait?

    /// Name the operation about to block. This only changes the diagnostic.
    public static func waitingFor(_ operation: String) {
        wait?.state.withLock {
            guard let last = $0.operations.indices.last else { return }
            $0.operations[last] = operation
        }
    }

    /// Name a nested wait. Pair this with `finishedWaiting()` in a defer.
    public static func beginWaiting(_ operation: String) {
        wait?.state.withLock {
            guard !$0.operations.isEmpty else { return }
            $0.operations.append(operation)
        }
    }

    /// Restore the caller's label.
    public static func finishedWaiting() {
        wait?.state.withLock { if $0.operations.count > 1 { $0.operations.removeLast() } }
    }

    public func provideScope(for test: Test, testCase: Test.Case?,
                             performing function: @Sendable () async throws -> Void) async throws {
        guard testCase != nil else { return try await function() }
        try await Self.run(test.name, seconds: 60, function)
    }

    /// The watchdog around one test case. Production always passes 60 from
    /// `provideScope`; only the watchdog's own tests pass a shorter bound.
    ///
    /// When it starts the clock: a `@MainActor` body first runs in a job on
    /// the main actor's serial queue. Calling `function` reaches that job
    /// only after a hop through the cooperative pool, so arming in a
    /// main-actor hop of our own would still charge the body for every job
    /// already queued behind that hop. Instead the arming job is enqueued
    /// here, immediately before `function` is called. It takes its place in
    /// the main actor's queue just ahead of the body's first job: it waits
    /// out the same queue uncharged and arms the timer right before the
    /// body's first line. Only jobs enqueued in the moment between the two
    /// (while Swift Testing's glue runs on the cooperative pool) still count.
    ///
    /// Nothing waits without a bound. If another test holds the main actor
    /// forever, this case never arms, but the holder armed before its own
    /// body began, so its watchdog fires and names it. A body that is not
    /// main-actor-isolated (a nonisolated suite) does not wait for the main
    /// actor, but it is armed the same way: as soon as the main actor is
    /// idle, which is at once in the common case, and after any queued
    /// main-actor work otherwise, each part of which has its own watchdog.
    /// The Swift gate's ten-minute deadline backs every case.
    package static func run(_ name: String, seconds: Int,
                            _ function: @Sendable () async throws -> Void) async throws {
        let wait = Wait()
        let watchdog = DispatchWorkItem {
            let operation = wait.state.withLock {
                $0.expired = !$0.operations.isEmpty
                return $0.operations.last
            }
            guard let operation else { return }
            TestStackCapture.fail(name, after: seconds, waitingFor: operation)
        }
        // DispatchWorkItem is thread-safe but not marked Sendable.
        nonisolated(unsafe) let timer = watchdog
        // A body that ends before this job runs has cancelled the item,
        // and a cancelled DispatchWorkItem never executes.
        Task { @MainActor in
            DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + .seconds(seconds), execute: timer)
        }
        defer { wait.finish(watchdog) }
        try await Self.$wait.withValue(wait) { try await function() }
    }
}
