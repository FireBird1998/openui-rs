import Darwin
import Dispatch
import Foundation
import Synchronization
import Testing

/// A wall-clock limit for cases that call AppKit synchronously. A Swift task
/// timeout cannot interrupt AppKit or a continuation that ignores cancellation.
struct NativeTestWatchdog: SuiteTrait, TestTrait, TestScoping {
    let isRecursive = true

    private final class Wait: Sendable {
        let operation = Mutex<String?>("the test body")
    }

    @TaskLocal private static var wait: Wait?

    static func waitingFor(_ operation: String) {
        wait?.operation.withLock { $0 = operation }
    }

    func provideScope(for test: Test, testCase: Test.Case?,
                      performing function: @Sendable () async throws -> Void) async throws {
        guard testCase != nil else { return try await function() }
        let wait = Wait()
        let name = test.name
        let watchdog = DispatchWorkItem {
            guard let operation = wait.operation.withLock({ $0 }) else { return }
            let message = "\(name): timed out after 60s waiting for \(operation)\n"
            FileHandle.standardError.write(Data(message.utf8))
            // Reporting a Swift Testing issue alone leaves a blocked runner
            // alive. The gate will report its missing summary as a failure.
            _exit(EXIT_FAILURE)
        }
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + .seconds(60), execute: watchdog)
        defer {
            wait.operation.withLock { $0 = nil }
            watchdog.cancel()
        }
        try await Self.$wait.withValue(wait) { try await function() }
    }
}
