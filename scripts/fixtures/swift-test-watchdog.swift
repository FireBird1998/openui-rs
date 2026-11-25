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

@Test(NativeTestWatchdog(interval: .seconds(1)))
func endlessProgress() async throws {
    var checkpoints = 0
    while true {
        try await Task.sleep(for: .milliseconds(130))
        NativeTestWatchdog.progress()
        checkpoints += 1
        print("Fixture checkpoint \(checkpoints)")
    }
}

@Test(NativeTestWatchdog(interval: .milliseconds(400)))
func nestedWaitStalls() {
    NativeTestWatchdog.beginWaiting("the Carousel fixture to reach its scroll offset")
    defer { NativeTestWatchdog.finishedWaiting() }
    func readLabels() {
        NativeTestWatchdog.beginWaiting("AppKit to read the fixture button labels")
        defer { NativeTestWatchdog.finishedWaiting() }
        NativeTestWatchdog.waitingFor("AppKit to retry fixture layout")
    }
    readLabels()
    block()
}

@Test(NativeTestWatchdog(interval: .milliseconds(1500)))
func chartPollAllowsLateResult() async throws {
    NativeTestWatchdog.progress()
    let deadline = NativeTestWatchdog.waitingForPoll(
        "the chart fixture result", for: .milliseconds(1800), grace: .milliseconds(50))
    try await Task.sleep(for: .milliseconds(1650))
    try #require(ContinuousClock.now < deadline)
}

@Test func repetitionCountParsing() throws {
    let raw = try #require(ProcessInfo.processInfo.environment["OPENUI_FIXTURE_REPETITION_COUNTS"])
    let inputs = try JSONDecoder().decode([String?].self, from: Data(raw.utf8))
    print("Fixture counts: " + inputs.map { String(SwiftTestLimits.repetitionCount($0)) }.joined(separator: ","))
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

@Test(NativeTestWatchdog(interval: .milliseconds(1750)))
func exitChildProgressCannotOutlastParent() async throws {
    let diagnostics = ExitTestDiagnostics(in: FileManager.default.temporaryDirectory, childSeconds: 1.5)
    defer { diagnostics.show() }
    NativeTestWatchdog.waitingFor("the exit fixture parent to get its child result")
    try await #require(processExitsWith: .success, observing: [\.standardErrorContent]) {
        [diagnostics = diagnostics as ExitTestDiagnostics] in
        try await NativeTestWatchdog(interval: .milliseconds(1500)).run(
            "exitChildProgressCannotOutlastParent()", waitingFor: "the exit fixture child to start", diagnostics: diagnostics
        ) {
            try await Task.sleep(for: .milliseconds(600))
            NativeTestWatchdog.progress()
            NativeTestWatchdog.waitingFor("the renewed exit fixture child")
            block()
        }
    }
}
