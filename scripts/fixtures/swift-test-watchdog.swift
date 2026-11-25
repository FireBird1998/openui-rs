import Dispatch
import Foundation
import SwiftTestSupport
import Testing

private func block() {
    DispatchSemaphore(value: 0).wait()
}

@Test(NativeTestWatchdog.displayCondition(for: "blockedDisplay()", watchdog: .init(interval: .milliseconds(400))) {
    block()
    return true
})
func blockedDisplay() {
    Issue.record("The test body must not run while the display probe is blocked")
}

@Test(NativeTestWatchdog(interval: .milliseconds(400)))
func namedOperationStalls() async {
    NativeTestWatchdog.waitingFor("Core to load the schema fixture")
    await withCheckedContinuation { (_: CheckedContinuation<Void, Never>) in block() }
}

@Test(NativeTestWatchdog(interval: .milliseconds(600)))
func continuousProgress() async throws {
    for _ in 0..<8 {
        try await Task.sleep(for: .milliseconds(130))
        NativeTestWatchdog.progress()
    }
}

@Test func stalledExitChild() async throws {
    let diagnostics = ExitTestDiagnostics(in: FileManager.default.temporaryDirectory)
    defer { diagnostics.show() }
    try await #require(processExitsWith: .success, observing: [\.standardErrorContent]) {
        [diagnostics = diagnostics as ExitTestDiagnostics] in
        await NativeTestWatchdog(interval: .milliseconds(400)).run(
            "stalledExitChild()", waitingFor: "the helper to return", diagnostics: diagnostics
        ) {
            block()
        }
    }
}
