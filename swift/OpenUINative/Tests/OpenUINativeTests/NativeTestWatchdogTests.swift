import Darwin
import Foundation
import Synchronization
import SwiftTestSupport
import Testing

/// The watchdog's own tests. A firing watchdog ends its process with
/// `_exit`, so every case runs in an exit-test child. They use a scaled
/// bound through `NativeTestWatchdog.run`; production's 60 s comes only
/// from the trait and is not changed here.
struct NativeTestWatchdogTests {
    /// The scaled stand-in for production's 60 s.
    static let bound = 3

    /// A main-actor blocker that started before the test was queued holds
    /// the main actor for three bounds, and a second one is queued while
    /// the test waits. The short test must not be charged for either.
    @Test func aTestWaitingForTheMainActorIsNotChargedForTheWait() async {
        let result = await #expect(processExitsWith: .success, observing: [\.standardErrorContent]) {
            try await NativeTestWatchdogTests.shortTestQueuedBehindABlocker()
        }
        if let result {
            #expect(!String(decoding: result.standardErrorContent, as: UTF8.self).contains("Swift test watchdog"))
        }
    }

    /// A body that itself runs past the bound still fires, with its name,
    /// its operation, the bound in the message and an in-process stack.
    @Test func aBodyThatRunsPastTheBoundFiresAndIsNamed() async throws {
        let stacks = FileManager.default.temporaryDirectory
            .appending(path: "watchdog-test-\(UUID())", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: stacks, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: stacks) }
        let result = await #expect(processExitsWith: .failure, observing: [\.standardErrorContent]) {
            [stacks = stacks.path as String] in
            setenv("OPENUI_SWIFT_TEST_STACKS", stacks + "/reports", 1)
            try await NativeTestWatchdog.run("overrunningBody", seconds: NativeTestWatchdogTests.bound) {
                await NativeTestWatchdogTests.likeSwiftTesting { @MainActor in
                    NativeTestWatchdog.waitingFor("a deliberate overrun")
                    // Finite, so a watchdog that never arms lets the child
                    // exit successfully and this test fails rather than hangs.
                    sleep(UInt32(NativeTestWatchdogTests.bound * 6))
                }
            }
        }
        let log = String(decoding: result?.standardErrorContent ?? [], as: UTF8.self)
        #expect(log.contains("Swift test watchdog: overrunningBody: timed out after 3s waiting for a deliberate overrun\n"))
        #expect(log.contains("Swift test watchdog: stack in \(stacks.path)/reports/"))
        let reports = (try? FileManager.default.contentsOfDirectory(atPath: stacks.appending(path: "reports").path)) ?? []
        #expect(reports.contains { $0.hasSuffix(".backtrace.txt") })
    }

    /// A test queued forever behind a hung test is covered by the hung
    /// test's own watchdog: the hung test is named, never the victim.
    @Test func aHungTestIsNamedNotTheTestQueuedBehindIt() async throws {
        let stacks = FileManager.default.temporaryDirectory
            .appending(path: "watchdog-test-\(UUID())", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: stacks, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: stacks) }
        let result = await #expect(processExitsWith: .failure, observing: [\.standardErrorContent]) {
            [stacks = stacks.path as String] in
            setenv("OPENUI_SWIFT_TEST_STACKS", stacks + "/reports", 1)
            try await NativeTestWatchdogTests.victimQueuedBehindAHungTest()
        }
        let log = String(decoding: result?.standardErrorContent ?? [], as: UTF8.self)
        #expect(log.contains("Swift test watchdog: hungTest: timed out after 3s waiting for a main actor that never returns\n"))
        #expect(!log.contains("queuedVictim"))
    }

    /// Swift Testing hands the trait a nonisolated `function` that reaches
    /// a `@MainActor` body only after running on the cooperative pool, so
    /// the body's first main-actor job is queued after the trait's own.
    /// Passing a `@MainActor` closure straight to `run` would skip that hop.
    @concurrent private static func likeSwiftTesting(_ body: @MainActor @Sendable () -> Void) async {
        precondition(pthread_main_np() == 0, "The stand-in for Swift Testing's glue ran on the main thread")
        await body()
    }

    private final class Mark: Sendable {
        private let instant = Mutex<ContinuousClock.Instant?>(nil)
        func set() { instant.withLock { $0 = .now } }
        var value: ContinuousClock.Instant? { instant.withLock { $0 } }

        func awaitSet() async throws {
            while value == nil { try await Task.sleep(for: .milliseconds(10)) }
        }
    }

    private static func shortTestQueuedBehindABlocker() async throws {
        let blocking = Mark()
        Task { @MainActor in
            blocking.set()
            sleep(UInt32(bound * 3))
        }
        try await blocking.awaitSet()
        let queued = ContinuousClock.now
        let bodyStarted = Mark()
        async let test: Void = NativeTestWatchdog.run("queuedShortTest", seconds: bound) {
            await likeSwiftTesting { @MainActor in bodyStarted.set() }
        }
        // A heavy neighbour joins the main actor's queue while the test
        // waits. The body was queued first, so it must not wait behind it.
        try await Task.sleep(for: .seconds(1))
        Task { @MainActor in sleep(UInt32(bound * 2)) }
        try await test
        // The case proves something only if the wait outlasted the bound.
        guard let started = bodyStarted.value, started - queued > .seconds(bound + 1) else {
            fputs("The short test did not wait longer than the bound for the main actor\n", stderr)
            exit(2)
        }
    }

    private static func victimQueuedBehindAHungTest() async throws {
        let hung = Mark()
        Task {
            try? await NativeTestWatchdog.run("hungTest", seconds: bound) {
                await likeSwiftTesting { @MainActor in
                    hung.set()
                    NativeTestWatchdog.waitingFor("a main actor that never returns")
                    sleep(UInt32(bound * 6))
                }
            }
        }
        try await hung.awaitSet()
        try await NativeTestWatchdog.run("queuedVictim", seconds: bound) {
            await likeSwiftTesting { @MainActor in }
        }
    }
}
