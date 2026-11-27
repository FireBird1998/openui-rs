// A watchdog for the body of an exit-test child process.
import Darwin
import Foundation

// The parent's Swift Testing time limit records an issue. Give the child
// its own watchdog so a stuck AppKit operation also ends the exit test.
// The diagnostic names the helper, as in "HtmlView helper PID 123 was still
// running at its deadline while probing the cache."
func withExitDeadline(_ helper: String, _ activity: String, by deadline: TimeInterval,
                      _ probe: @MainActor @Sendable () async throws -> Void) async throws {
    let watchdog = Task.detached {
        let remaining = max(0, deadline - ProcessInfo.processInfo.systemUptime)
        try await Task.sleep(for: .seconds(remaining))
        let diagnostic = "\(helper) helper PID \(getpid()) was still running at its deadline while \(activity).\n"
        try? FileHandle.standardError.write(contentsOf: Data(diagnostic.utf8))
        Darwin.exit(EXIT_FAILURE)
    }
    defer { watchdog.cancel() }
    try await probe()
}
