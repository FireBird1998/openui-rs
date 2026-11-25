import Darwin
import Dispatch
import Foundation
import Synchronization
import Testing

/// An inactivity limit for cases that can block AppKit or wait on a continuation.
public struct NativeTestWatchdog: SuiteTrait, TestTrait, TestScoping {
    public let isRecursive = true
    private let interval: Duration

    private struct State {
        var operation: String
        var deadline: ContinuousClock.Instant
        var armed = true
    }

    private final class Wait: Sendable {
        let state: Mutex<State>
        let initialOperation: String
        let interval: Duration

        init(_ operation: String, interval: Duration) {
            state = Mutex(State(operation: operation, deadline: .now + interval))
            initialOperation = operation
            self.interval = interval
        }
    }

    @TaskLocal private static var wait: Wait?

    public init(seconds: Int = 150) {
        interval = .seconds(seconds)
    }

    // The duration overload also lets the regression fixture exercise expiry quickly.
    public init(interval: Duration) {
        self.interval = interval
    }

    public static func waitingFor(_ operation: String, allowing allowance: Duration? = nil) {
        wait?.state.withLock {
            $0.operation = operation
            if let allowance { $0.deadline = .now + allowance }
        }
    }

    public static func finishedWaiting() {
        if let wait { waitingFor(wait.initialOperation) }
    }

    /// Call after completing a unit of work. A poll that finds no change is not progress.
    public static func progress() {
        guard let wait else { return }
        wait.state.withLock { $0.deadline = .now + wait.interval }
        TestStackCapture.reportProgress()
    }

    public func provideScope(for test: Test, testCase: Test.Case?,
                             performing function: @Sendable () async throws -> Void) async throws {
        guard testCase != nil else { return try await function() }
        try await run(test.name, waitingFor: "Swift Testing to enter or complete \(test.name)", performing: function)
    }

    // Conditions are evaluated before TestScoping and can block entering the main actor.
    public static func displayCondition(for name: String, watchdog: Self = Self(),
                                        checking display: @escaping @MainActor @Sendable () -> Bool) -> ConditionTrait {
        .enabled("No macOS display is available") {
            await watchdog.run(name, waitingFor: "AppKit to enumerate macOS displays") {
                await display()
            }
        }
    }

    // Exit-test helpers also run outside the parent's TestScoping traits.
    public func run<T: Sendable>(_ name: String, waitingFor operation: String, diagnostics: ExitTestDiagnostics? = nil,
                                 performing function: @Sendable () async throws -> T) async rethrows -> T {
        let wait = Wait(operation, interval: interval)
        let queue = DispatchQueue.global(qos: .userInitiated)
        let timer = DispatchSource.makeTimerSource(queue: queue)
        let capture = TestStackCapture.register(name)
        @Sendable func schedule(after delay: Duration) {
            let components = delay.components
            let whole = Int(clamping: components.seconds)
            guard whole < Int.max / 1_000_000_000 else {
                timer.schedule(deadline: .distantFuture)
                return
            }
            let fraction = Int(clamping: components.attoseconds / 1_000_000_000)
            timer.schedule(deadline: .now() + .nanoseconds(whole * 1_000_000_000 + fraction))
        }
        timer.setEventHandler {
            let expired = wait.state.withLock { state -> String? in
                guard state.armed else { return nil }
                let remaining = ContinuousClock.now.duration(to: state.deadline)
                if remaining > .zero {
                    schedule(after: remaining)
                    return nil
                }
                state.armed = false
                return state.operation
            }
            if let expired {
                TestStackCapture.fail(name, waitingFor: expired, interval: interval, diagnostics: diagnostics)
            }
        }
        schedule(after: max(.zero, interval))
        timer.activate()
        defer {
            wait.state.withLock { $0.armed = false }
            timer.cancel()
            TestStackCapture.unregister(capture)
        }
        return try await Self.$wait.withValue(wait) { try await function() }
    }

}
