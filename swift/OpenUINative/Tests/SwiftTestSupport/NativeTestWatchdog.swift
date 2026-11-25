import Darwin
import Dispatch
import Foundation
import Synchronization
import Testing

/// A wall-clock limit for cases that call AppKit synchronously. A Swift task
/// timeout cannot interrupt AppKit or a continuation that ignores cancellation.
public struct NativeTestWatchdog: SuiteTrait, TestTrait, TestScoping {
    public let isRecursive = true
    private let seconds: Int

    private final class Wait: Sendable {
        let operation: Mutex<String?>

        init(_ operation: String) {
            self.operation = Mutex(operation)
        }
    }

    @TaskLocal private static var wait: Wait?

    public init(seconds: Int = 60) {
        self.seconds = seconds
    }

    public static func waitingFor(_ operation: String) {
        wait?.operation.withLock { $0 = operation }
    }

    public static func finishedWaiting() {
        waitingFor("the test body")
    }

    public func provideScope(for test: Test, testCase: Test.Case?,
                             performing function: @Sendable () async throws -> Void) async throws {
        guard testCase != nil else { return try await function() }
        try await run(test.name, waitingFor: "the test body", performing: function)
    }

    // Swift Testing's exit-test helper is a separate process and does not run
    // the parent's TestScoping traits.
    public func run(_ name: String, waitingFor operation: String,
                    until deadline: DispatchTime? = nil,
                    performing function: @Sendable () async throws -> Void) async throws {
        let wait = Wait(operation)
        let watchdog = DispatchWorkItem {
            guard let operation = wait.operation.withLock({ $0 }) else { return }
            TestStackCapture.fail("\(name): timed out after \(seconds)s waiting for \(operation)")
        }
        DispatchQueue.global(qos: .userInitiated).asyncAfter(
            deadline: deadline ?? .now() + .seconds(seconds), execute: watchdog)
        defer {
            wait.operation.withLock { $0 = nil }
            watchdog.cancel()
        }
        TestStackCapture.listenForGateRequests()
        try await Self.$wait.withValue(wait) { try await function() }
    }
}
