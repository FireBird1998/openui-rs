import Darwin
import Dispatch
import Foundation
import Synchronization
import Testing

/// A timer for cases that can block AppKit or wait on a continuation.
public struct NativeTestWatchdog: SuiteTrait, TestTrait, TestScoping {
    public let isRecursive = true
    private let interval: Duration
    private let renewable: Bool

    public static var native: Self {
        Self(interval: .seconds(SwiftTestLimits.shared.nativeSeconds), renewable: false)
    }

    public static var exitChild: Self {
        Self(seconds: SwiftTestLimits.shared.exitChildSeconds)
    }

    private struct State {
        var operations: [String]
        var deadline: ContinuousClock.Instant
        var armed = true
    }

    private final class Wait: Sendable {
        let state: Mutex<State>
        let interval: Duration
        let ceiling: ContinuousClock.Instant?

        init(_ operation: String, interval: Duration, ceiling: ContinuousClock.Instant?) {
            let deadline = ContinuousClock.now + interval
            state = Mutex(State(operations: [operation], deadline: min(deadline, ceiling ?? deadline)))
            self.interval = interval
            self.ceiling = ceiling
        }

        func deadline(after allowance: Duration) -> ContinuousClock.Instant {
            let deadline = ContinuousClock.now + allowance
            return min(deadline, ceiling ?? deadline)
        }
    }

    @TaskLocal private static var wait: Wait?

    public init(seconds: Int = SwiftTestLimits.shared.ordinarySeconds) {
        self.init(interval: .seconds(seconds))
    }

    // The duration overload also lets the regression fixture exercise expiry quickly.
    public init(interval: Duration, renewable: Bool = true) {
        self.interval = interval
        self.renewable = renewable
    }

    public static func waitingFor(_ operation: String, allowing allowance: Duration? = nil) {
        guard let wait else { return }
        wait.state.withLock {
            $0.operations[$0.operations.count - 1] = operation
            if let allowance { $0.deadline = wait.deadline(after: allowance) }
        }
    }

    public static func beginWaiting(_ operation: String) {
        wait?.state.withLock { $0.operations.append(operation) }
    }

    public static func finishedWaiting() {
        wait?.state.withLock {
            if $0.operations.count > 1 { $0.operations.removeLast() }
        }
    }

    public static func waitingForPoll(_ operation: String, for timeout: Duration,
                                      grace: Duration = .seconds(SwiftTestLimits.shared.schedulingSeconds))
        -> ContinuousClock.Instant {
        let deadline = ContinuousClock.now + timeout
        waitingFor(operation, allowing: timeout + grace)
        return deadline
    }

    /// Call after completing a unit of work. A poll that finds no change is not progress.
    public static func progress() {
        guard let wait else { return }
        wait.state.withLock { $0.deadline = wait.deadline(after: wait.interval) }
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
        let start = ContinuousClock.now
        let ceiling = diagnostics.map { start + $0.childTimeRemaining }
            ?? (renewable ? nil : start + interval)
        let wait = Wait(operation, interval: interval, ceiling: ceiling)
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
                return state.operations.last
            }
            if let expired {
                TestStackCapture.fail(name, waitingFor: expired, elapsed: start.duration(to: .now), diagnostics: diagnostics)
            }
        }
        schedule(after: max(.zero, ContinuousClock.now.duration(to: wait.state.withLock { $0.deadline })))
        timer.activate()
        defer {
            wait.state.withLock { $0.armed = false }
            timer.cancel()
            TestStackCapture.unregister(capture)
        }
        return try await Self.$wait.withValue(wait) { try await function() }
    }

}
