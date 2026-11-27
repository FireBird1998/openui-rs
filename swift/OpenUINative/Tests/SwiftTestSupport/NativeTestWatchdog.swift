import Darwin
import Dispatch
import Synchronization
import Testing

/// A fixed wall-clock deadline for each test case. It can stop AppKit calls
/// and continuations that do not respond to Swift task cancellation.
///
/// The limit means "60 s from the body's first line", not 60 s from when
/// Swift Testing starts the case. Most suites are `@MainActor`, and their
/// bodies wait for the main actor behind other tests' work, so the clock
/// starts in a main-actor job queued just before the body's own first
/// main-actor job (see `run`). Once started it never pauses: when the body
/// suspends and later waits for the main actor again, that wait counts.
///
/// An outer limit, counted from when the case starts, backs it up for a
/// case whose clock never starts because the main actor never reaches it.
public struct NativeTestWatchdog: SuiteTrait, TestTrait, TestScoping {
    public let isRecursive = true

    /// A case's limit. The set is closed: the standard 60 s, or a named
    /// longer limit declared here with the measurement it comes from.
    /// Nothing can lengthen, shorten or renew a limit at run time.
    public struct Limit: Sendable {
        let seconds: Int

        /// The limit for every case unless one of the names below applies.
        public static let standard = Limit(seconds: 60)

        // The three HtmlView limits below are for display-only cases, which
        // the headless gate skips. Each keeps the 2-minute allowance its
        // test already had, a deliberately loose match to its runner time.

        /// `HtmlViewTests.httpCacheIsSeparateAcrossViewsAndAppProcesses`
        /// mounts three views, then probes the cache from a fresh app process
        /// that has its own 60 s deadline. It has had a 2-minute `.timeLimit`
        /// since #254 part A for that reason. On GitHub's runner it passed
        /// after 9.1 s from case start in run 46036129851.
        public static let htmlViewCacheAcrossAppProcesses = Limit(seconds: 120)

        /// `HtmlViewTests.spinningPageKeepsHostResponsiveReleasesViewAndExitsWhenHostExits`
        /// runs the shared runaway check, then a helper host with its own
        /// 60 s deadline, then waits up to 10 s for WebContent to exit. It has
        /// had a 2-minute `.timeLimit`. On GitHub's runner it passed after
        /// 5.0 s from case start in run 46036129851.
        public static let htmlViewSpinningPage = Limit(seconds: 120)

        /// `HtmlViewTests.allocatingPageLeavesHostResponsiveAndExitsWhenDismissed`
        /// runs the same runaway check as the spinning case without the
        /// helper host, plus up to 10 s for WebContent to exit on dismissal.
        /// It had no limit; the spinning case passing in 2 minutes bounds it.
        /// On GitHub's runner it passed after 2.9 s from case start in run
        /// 46036129851.
        public static let htmlViewAllocatingPage = Limit(seconds: 120)

        /// `FunctionReplayTests.everyRecordedPairReplaysThroughTheRuntime`
        /// replays about 3,125 recorded pairs through `LiveProgram.show`, and
        /// each `show` comes back to the main actor, so every re-entry wait
        /// behind other tests counts against its body clock. On GitHub's
        /// runner its body ran over 60 s and timed out in the
        /// ci/mainrisk-backstop-a run, and it passed 96.8 s from case start
        /// in run 46036129851. 180 s is provisional, from those two samples:
        /// the test prints "Function replay body: N s" on every run, so set
        /// it from runner logs of the next runs.
        public static let functionReplay = Limit(seconds: 180)

        /// How long after the case starts the outer limit fires. Queue waits
        /// for the main actor of up to about 57 s were measured on GitHub's
        /// runner in run 46036129851, so a standard case can legitimately
        /// finish about 60 + 57 ≈ 117 s after it starts; 180 s is about 1.5×
        /// that. A named limit gets the same allowance on top of its own
        /// length. This is a starting value: re-measure it from the first
        /// runner runs after the outer limit lands.
        var outerSeconds: Int { (seconds + 60) * 3 / 2 }
    }

    let limit: Limit

    /// The standard 60 s limit.
    public init() { limit = .standard }

    /// A named longer limit; see `Limit`.
    public init(_ limit: Limit) { self.limit = limit }

    private struct State {
        var operations = ["the test body"]
        var expired = false
        var armed: ContinuousClock.Instant?
    }

    private final class Wait: Sendable {
        let state = Mutex(State())

        func finish(_ timers: [DispatchWorkItem]) {
            let expired = state.withLock {
                $0.operations.removeAll()
                return $0.expired
            }
            for timer in timers { timer.cancel() }
            // Once a timer wins, keep the runner alive for capture and
            // prevent a late body completion from reporting a passing case.
            if expired { DispatchSemaphore(value: 0).wait() }
        }

        /// Claim the case for a firing timer: the operation it was waiting
        /// for, or nil if the body already finished.
        func expire() -> (operation: String, armed: ContinuousClock.Instant?)? {
            state.withLock { state in
                state.expired = !state.operations.isEmpty
                return state.operations.last.map { ($0, state.armed) }
            }
        }
    }

    @TaskLocal private static var wait: Wait?

    /// Name the operation about to block. This only changes the diagnostic.
    public static func waitingFor(_ operation: String) {
        wait?.state.withLock {
            guard let last = $0.operations.indices.last else { return }
            $0.operations[last] = operation
        }
    }

    /// Name a nested wait. Pair this with `finishedWaiting()` in a defer.
    public static func beginWaiting(_ operation: String) {
        wait?.state.withLock {
            guard !$0.operations.isEmpty else { return }
            $0.operations.append(operation)
        }
    }

    /// Restore the caller's label.
    public static func finishedWaiting() {
        wait?.state.withLock { if $0.operations.count > 1 { $0.operations.removeLast() } }
    }

    /// Whether the current case's limit has started. Nil outside a case.
    /// Only the watchdog's own tests read it, to check that the clock is
    /// running by a `@MainActor` body's first line.
    package static var clockStarted: Bool? {
        wait?.state.withLock { $0.armed != nil }
    }

    public func provideScope(for test: Test, testCase: Test.Case?,
                             performing function: @Sendable () async throws -> Void) async throws {
        guard testCase != nil else { return try await function() }
        try await Self.run(test.name, seconds: limit.seconds, outerSeconds: limit.outerSeconds, function)
    }

    /// The watchdog around one test case. Production passes a `Limit`'s
    /// values from `provideScope`; only the watchdog's own tests pass
    /// shorter bounds.
    ///
    /// When it starts the clock: a `@MainActor` body first runs in a job on
    /// the main actor's serial queue. Calling `function` reaches that job
    /// only after a hop through the cooperative pool, so arming in a
    /// main-actor hop of our own would still charge the body for every job
    /// already queued behind that hop. Instead the arming job is enqueued
    /// here, immediately before `function` is called. It takes its place in
    /// the main actor's queue just ahead of the body's first job: it waits
    /// out the same queue uncharged and arms the timer right before the
    /// body's first line. Only jobs enqueued in the moment between the two
    /// (while Swift Testing's glue runs on the cooperative pool) still count.
    ///
    /// That order rests on how the runtime behaves, not on a documented
    /// language guarantee:
    /// - Since Swift 6.0, `Task.init` takes an `@isolated(any)` operation
    ///   (SE-0431), so `Task { @MainActor in … }` enqueues its first job on
    ///   the main executor synchronously, before `function()` is called.
    /// - On Darwin the main executor is the main dispatch queue, which runs
    ///   jobs in FIFO order; priority boosts the draining thread but never
    ///   reorders it.
    /// - `provideScope` and `function` run off the main actor (tools 6.0,
    ///   no default main-actor isolation, no `NonisolatedNonsendingByDefault`).
    ///   If Swift Testing ever called in from the main actor, a main-actor
    ///   body could run inline ahead of its arming job.
    /// `NativeTestWatchdogTests` fails if the clock is not running by a
    /// `@MainActor` body's first line, so such a change announces itself.
    ///
    /// The outer limit is armed at once, here, and counts from the case's
    /// start. It catches the cases the body clock cannot: a case queued
    /// forever behind a main-actor holder that has no watchdog of its own,
    /// so it never arms, and a nonisolated body, which runs at once on the
    /// cooperative pool but is armed only when the main actor reaches its
    /// arming job. Its message says it is the outer limit and that the
    /// body may never have started, so it is not read as a body overrun.
    /// The Swift gate's ten-minute deadline backs every case.
    package static func run(_ name: String, seconds: Int, outerSeconds: Int,
                            _ function: @Sendable () async throws -> Void) async throws {
        let wait = Wait()
        let watchdog = DispatchWorkItem {
            guard let (operation, _) = wait.expire() else { return }
            TestStackCapture.fail(name, "\(name): timed out after \(seconds)s waiting for \(operation)")
        }
        let start = ContinuousClock.now
        let outer = DispatchWorkItem {
            guard let (operation, armed) = wait.expire() else { return }
            let clock = armed.map {
                "its \(seconds) s limit started only \(($0 - start).components.seconds) s in, after waiting for the main actor"
            } ?? "its \(seconds) s limit never started, so its body may never have reached the main actor"
            TestStackCapture.fail(name, "outer limit: \(name) did not finish within \(outerSeconds) s of starting; \(clock) (waiting for \(operation))")
        }
        // DispatchWorkItem is thread-safe but not marked Sendable.
        nonisolated(unsafe) let timer = watchdog
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + .seconds(outerSeconds), execute: outer)
        // A body that ends before this job runs has cancelled the item,
        // and a cancelled DispatchWorkItem never executes.
        Task { @MainActor in
            wait.state.withLock { $0.armed = .now }
            DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + .seconds(seconds), execute: timer)
        }
        defer { wait.finish([watchdog, outer]) }
        try await Self.$wait.withValue(wait) { try await function() }
    }
}
