import Darwin
import Dispatch
import Synchronization
import Testing

/// A fixed wall-clock deadline for each test case. It can stop AppKit calls
/// and continuations that do not respond to Swift task cancellation.
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
        let wait = Wait()
        let name = test.name
        let watchdog = DispatchWorkItem {
            let operation = wait.state.withLock {
                $0.expired = !$0.operations.isEmpty
                return $0.operations.last
            }
            guard let operation else { return }
            TestStackCapture.fail(name, waitingFor: operation)
        }
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + .seconds(60), execute: watchdog)
        defer { wait.finish(watchdog) }
        try await Self.$wait.withValue(wait) { try await function() }
    }
}
