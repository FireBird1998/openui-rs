import Darwin
import Foundation
import SwiftTestSupport
import Testing

// `ExitStamp` decides HtmlViewTests' display-only WebContent exit checks.
// These check it with plain child processes, so they need no display.
@Suite(NativeTestWatchdog()) @MainActor struct ProcessExitStampTests {
    private typealias WebProcess = CollectionDrawingTests.HtmlViewTests.WebProcess

    private func launch(_ shell: String) throws -> Process {
        let child = Process()
        child.executableURL = URL(fileURLWithPath: "/bin/sh")
        child.arguments = ["-c", shell]
        try child.run()
        return child
    }

    /// The defect the stamp fixes: a process that exits in time, checked only
    /// after the main actor was held past the deadline, still counts as in time.
    @Test func anExitInTimeCountsWhenTheCheckWakesLate() async throws {
        let child = try launch("exec /bin/sleep 0.2")
        defer { if child.isRunning { kill(child.processIdentifier, SIGKILL) } }
        let process = try #require(WebProcess(child.processIdentifier))
        let exit = ExitStamp(child.processIdentifier)
        child.waitUntilExit()
        // The child has exited by now; a deadline a second later is in time
        // for it, whatever its start-up took. Then wake past that deadline, as
        // a late main-actor wake would, before checking. This suspends rather
        // than blocking the main thread, which other suites share.
        let deadline = ProcessInfo.processInfo.systemUptime + 1
        try await Task.sleep(for: .seconds(1.5))
        try #require(ProcessInfo.processInfo.systemUptime > deadline)
        let outcome = await exit.outcome(by: deadline, exited: { process.hasExited })
        guard case .exited(let at) = outcome else {
            Issue.record("Expected an exit stamp, got \(outcome)")
            return
        }
        #expect(at <= deadline, "Stamped \(at - deadline) s past the deadline")
    }

    /// A process already gone when the watch starts is stamped at once, at
    /// the watch's start: never earlier than the exit, and no hang.
    @Test func aProcessThatHasAlreadyExitedIsStampedAtTheWatchsStart() async throws {
        let child = try launch("exit 0")
        child.waitUntilExit()
        let started = ProcessInfo.processInfo.systemUptime
        let exit = ExitStamp(child.processIdentifier)
        let outcome = await exit.outcome(by: started + 30, exited: { true })
        guard case .exited(let at) = outcome else {
            Issue.record("Expected an exit stamp, got \(outcome)")
            return
        }
        #expect(at >= started)
    }

    /// A process that counts as exited at the deadline but whose stamp never
    /// comes (here, a live process standing in for a reused pid) fails by name
    /// a recheck later, not at the watchdog.
    @Test func anExitWithoutAStampIsReportedAfterARecheck() async throws {
        let child = try launch("exec /bin/sleep 600")
        defer { if child.isRunning { kill(child.processIdentifier, SIGKILL) } }
        let exit = ExitStamp(child.processIdentifier)
        let deadline = ProcessInfo.processInfo.systemUptime + 0.3
        let outcome = await exit.outcome(by: deadline, exited: { true })
        guard case .exitedWithoutStamp(let at) = outcome else {
            Issue.record("Expected an exit without a stamp, got \(outcome)")
            return
        }
        #expect(at >= deadline)
        #expect(child.isRunning)
    }

    /// A process still running at a check made at or after the deadline fails
    /// then, without waiting for an exit that may never come.
    @Test func aProcessRunningAtTheDeadlineIsReportedThen() async throws {
        let child = try launch("exec /bin/sleep 600")
        defer { if child.isRunning { kill(child.processIdentifier, SIGKILL) } }
        let process = try #require(WebProcess(child.processIdentifier))
        let exit = ExitStamp(child.processIdentifier)
        let deadline = ProcessInfo.processInfo.systemUptime + 0.3
        let outcome = await exit.outcome(by: deadline, exited: { process.hasExited })
        guard case .runningAtDeadline(let at) = outcome else {
            Issue.record("Expected the process to be running at the deadline, got \(outcome)")
            return
        }
        #expect(at >= deadline)
        #expect(process.isRunning)
    }
}
