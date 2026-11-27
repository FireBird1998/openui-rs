import Darwin
import Foundation
import Synchronization
@_spi(WatchdogTests) import SwiftTestSupport
import Testing

/// The watchdog's own tests. A firing watchdog ends its process with
/// `_exit`, so every case runs in an exit-test child. They use a scaled
/// bound through `NativeTestWatchdog.run`; production's 60 s comes only
/// from the trait and is not changed here.
struct NativeTestWatchdogTests {
    /// The scaled stand-in for production's 60 s.
    static let bound = 3
    /// The scaled stand-in for production's 180 s outer limit, at the same
    /// 3× ratio.
    static let outer = bound * 3

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
            try await NativeTestWatchdog.run("overrunningBody", seconds: NativeTestWatchdogTests.bound,
                                             outerSeconds: NativeTestWatchdogTests.outer) {
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
        #expect(!log.contains("outer limit"))
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

    /// A main-actor holder with no watchdog of its own blocks the main actor
    /// past the outer limit, so the queued test's body clock never starts.
    /// The outer limit fires, names the queued test, says it is the outer
    /// limit and captures a stack. Without it the holder would finish at
    /// six bounds and the child would exit successfully.
    @Test func aTestQueuedBehindAnUnwatchedHolderIsCaughtByTheOuterLimit() async throws {
        let stacks = FileManager.default.temporaryDirectory
            .appending(path: "watchdog-test-\(UUID())", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: stacks, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: stacks) }
        let result = await #expect(processExitsWith: .failure, observing: [\.standardErrorContent]) {
            [stacks = stacks.path as String] in
            setenv("OPENUI_SWIFT_TEST_STACKS", stacks + "/reports", 1)
            try await NativeTestWatchdogTests.unwatchedHolderThen("queuedBehindUnwatchedHolder") {
                await NativeTestWatchdogTests.likeSwiftTesting { @MainActor in }
            }
        }
        let log = String(decoding: result?.standardErrorContent ?? [], as: UTF8.self)
        #expect(log.contains(
            "Swift test watchdog: outer limit: queuedBehindUnwatchedHolder did not finish within 9 s of starting; "
                + "its 3 s limit never started, so its body may never have reached the main actor "
                + "(waiting for the test body)\n"))
        #expect(!log.contains("timed out after"))
        #expect(log.contains("Swift test watchdog: stack in \(stacks.path)/reports/"))
        let reports = (try? FileManager.default.contentsOfDirectory(atPath: stacks.appending(path: "reports").path)) ?? []
        #expect(reports.contains { $0.hasSuffix(".backtrace.txt") })
    }

    /// A healthy case that waits two bounds behind an unwatched holder and
    /// then runs for one bound finishes inside the outer limit. The child
    /// stays alive until well past the outer limit, so a backstop that was
    /// not cancelled when the case finished would still fire.
    @Test func aHealthyLongQueueWaitIsNotTouchedByTheOuterLimit() async {
        let result = await #expect(processExitsWith: .success, observing: [\.standardErrorContent]) {
            try await NativeTestWatchdogTests.healthyCaseAfterALongWait()
        }
        if let result {
            #expect(!String(decoding: result.standardErrorContent, as: UTF8.self).contains("Swift test watchdog"))
        }
    }

    /// A nonisolated body runs at once on the cooperative pool, but its
    /// clock starts only when the main actor runs the arming job. With an
    /// unwatched holder on the main actor, the outer limit catches it.
    @Test func aNonisolatedBodyWhoseClockCannotStartIsCaughtByTheOuterLimit() async throws {
        let result = await #expect(processExitsWith: .failure, observing: [\.standardErrorContent]) {
            try await NativeTestWatchdogTests.unwatchedHolderThen("hangingNonisolatedBody") {
                NativeTestWatchdog.waitingFor("a nonisolated hang")
                sleep(UInt32(NativeTestWatchdogTests.bound * 6))
            }
        }
        let log = String(decoding: result?.standardErrorContent ?? [], as: UTF8.self)
        #expect(log.contains(
            "Swift test watchdog: outer limit: hangingNonisolatedBody did not finish within 9 s of starting; "
                + "its 3 s limit never started, so its body may never have reached the main actor "
                + "(waiting for a nonisolated hang)\n"))
        #expect(!log.contains("timed out after"))
    }

    /// With the main actor free, a nonisolated body is armed at once and
    /// its own limit fires first, as an ordinary overrun.
    @Test func aNonisolatedBodyWithAFreeMainActorFiresAtItsOwnLimit() async throws {
        let result = await #expect(processExitsWith: .failure, observing: [\.standardErrorContent]) {
            try await NativeTestWatchdog.run("overrunningNonisolatedBody", seconds: NativeTestWatchdogTests.bound,
                                             outerSeconds: NativeTestWatchdogTests.outer) {
                NativeTestWatchdog.waitingFor("a nonisolated overrun")
                sleep(UInt32(NativeTestWatchdogTests.bound * 6))
            }
        }
        let log = String(decoding: result?.standardErrorContent ?? [], as: UTF8.self)
        #expect(log.contains("Swift test watchdog: overrunningNonisolatedBody: timed out after 3s waiting for a nonisolated overrun\n"))
        #expect(!log.contains("outer limit"))
    }

    /// The body clock must already be running on a `@MainActor` body's
    /// first line. This runs through the real trait and Swift Testing's own
    /// glue. It fails if the arming job ever runs after the body has begun,
    /// for example if a change of isolation defaults (default main-actor
    /// isolation, `NonisolatedNonsendingByDefault`) made Swift Testing call
    /// a main-actor body inline, ahead of the job. See `NativeTestWatchdog.run`.
    @Test(NativeTestWatchdog()) @MainActor func theClockIsRunningOnAMainActorBodysFirstLine() {
        #expect(NativeTestWatchdog.clockStarted == true,
                "The watchdog's arming job ran after the body began, so a hung main-actor body would never be armed")
    }

    /// The same check with the main actor busy when the case starts, so the
    /// arming job and the body's first job both queue behind a holder and
    /// their order in that queue decides the result.
    @Test func theClockIsRunningOnTheFirstLineOfABodyQueuedBehindAHolder() async {
        await #expect(processExitsWith: .success) {
            try await NativeTestWatchdogTests.firstLineOfAQueuedBody()
        }
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
        // This case waits about as long as `outer`, so it gets an outer limit
        // it cannot reach and tests the body clock alone. A healthy wait
        // under the outer limit has its own case.
        async let test: Void = NativeTestWatchdog.run("queuedShortTest", seconds: bound, outerSeconds: bound * 5) {
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
            try? await NativeTestWatchdog.run("hungTest", seconds: bound, outerSeconds: outer) {
                await likeSwiftTesting { @MainActor in
                    hung.set()
                    NativeTestWatchdog.waitingFor("a main actor that never returns")
                    sleep(UInt32(bound * 6))
                }
            }
        }
        try await hung.awaitSet()
        try await NativeTestWatchdog.run("queuedVictim", seconds: bound, outerSeconds: outer) {
            await likeSwiftTesting { @MainActor in }
        }
    }

    /// Hold the main actor for six bounds with no watchdog, as a test
    /// outside the trait would, then run `name` under the watchdog.
    private static func unwatchedHolderThen(_ name: String,
                                            _ body: @Sendable () async throws -> Void) async throws {
        let holding = Mark()
        Task { @MainActor in
            holding.set()
            sleep(UInt32(bound * 6))
        }
        try await holding.awaitSet()
        try await NativeTestWatchdog.run(name, seconds: bound, outerSeconds: outer, body)
    }

    private static func healthyCaseAfterALongWait() async throws {
        let holding = Mark()
        Task { @MainActor in
            holding.set()
            sleep(UInt32(bound * 2))
        }
        try await holding.awaitSet()
        let queued = ContinuousClock.now
        let bodyStarted = Mark()
        try await NativeTestWatchdog.run("healthyLongWait", seconds: bound, outerSeconds: outer) {
            await likeSwiftTesting { @MainActor in
                bodyStarted.set()
                sleep(UInt32(bound - 2))
            }
        }
        // The case proves something only if it waited longer than the body
        // limit and the whole case ran past the body limit too.
        guard let started = bodyStarted.value, started - queued > .seconds(bound + 1),
              ContinuousClock.now - queued > .seconds(bound * 2) else {
            fputs("The healthy case did not wait long enough for the main actor\n", stderr)
            exit(2)
        }
        // Outlive the outer limit with room for timer leeway.
        try await Task.sleep(until: queued + .seconds(outer + bound))
    }

    private static func firstLineOfAQueuedBody() async throws {
        let holding = Mark()
        Task { @MainActor in
            holding.set()
            sleep(2)
        }
        try await holding.awaitSet()
        try await NativeTestWatchdog.run("queuedFirstLine", seconds: bound * 5, outerSeconds: bound * 10) {
            await likeSwiftTesting { @MainActor in
                guard NativeTestWatchdog.clockStarted == true else {
                    fputs("The watchdog's arming job ran after the queued body began\n", stderr)
                    exit(3)
                }
            }
        }
    }
}
