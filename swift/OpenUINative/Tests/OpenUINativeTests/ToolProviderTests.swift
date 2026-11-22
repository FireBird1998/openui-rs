// The Swift driver for the tool provider (#92), against a fake provider and a
// clock the test moves. These check the driver's wiring, concurrency,
// cancellation and timer. When a query runs, reruns or refreshes is the
// session's to decide and is tested in Rust; it isn't re-tested here.
import AppKit
import OpenUIFFI
@testable import OpenUINative
import SwiftUI
import Synchronization
import Testing

/// A monotonic clock that only moves when the test advances it.
final class ManualClock: Clock, Sendable {
    struct Instant: InstantProtocol {
        var offset: Duration
        func advanced(by duration: Duration) -> Instant { Instant(offset: offset + duration) }
        func duration(to other: Instant) -> Duration { other.offset - offset }
        static func < (a: Instant, b: Instant) -> Bool { a.offset < b.offset }
    }

    private struct Sleeper {
        let deadline: Instant
        let continuation: CheckedContinuation<Void, any Error>
    }

    private struct State {
        var now = Instant(offset: .zero)
        var next = 0
        var sleepers: [Int: Sleeper] = [:]
        var requested: [Instant] = []
    }

    private let state = Mutex(State())

    var now: Instant { state.withLock { $0.now } }
    var minimumResolution: Duration { .milliseconds(1) }

    func sleep(until deadline: Instant, tolerance: Duration?) async throws {
        let id = state.withLock { $0.next += 1; return $0.next }
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
                let immediate: Result<Void, any Error>? = state.withLock { state in
                    state.requested.append(deadline)
                    if Task.isCancelled { return .failure(CancellationError()) }
                    if deadline <= state.now { return .success(()) }
                    state.sleepers[id] = Sleeper(deadline: deadline, continuation: continuation)
                    return nil
                }
                if let immediate { continuation.resume(with: immediate) }
            }
        } onCancel: {
            let sleeper = state.withLock { state in
                state.sleepers.removeValue(forKey: id)
            }
            sleeper?.continuation.resume(throwing: CancellationError())
        }
    }

    /// The deadlines someone is sleeping until, in milliseconds since zero.
    var deadlines: [Int64] {
        state.withLock { $0.sleepers.values.map { Int64($0.deadline.offset / .milliseconds(1)) }.sorted() }
    }

    var requestedDeadlines: [Int64] {
        state.withLock { $0.requested.map { Int64($0.offset / .milliseconds(1)) } }
    }

    /// Moves the clock to `ms` milliseconds and wakes every sleeper now due.
    func advance(to ms: Int64) {
        let due = state.withLock { state in
            state.now = Instant(offset: .milliseconds(ms))
            let due = state.sleepers.filter { $0.value.deadline <= state.now }
            for id in due.keys { state.sleepers[id] = nil }
            return due.values.map(\.continuation)
        }
        for continuation in due { continuation.resume() }
    }
}

/// A tool provider whose calls wait until the test answers them.
final class FakeTools: Sendable {
    struct Call: Sendable {
        let id: Int
        let name: String
        let arguments: JSONValue?
    }

    private struct State {
        var next = 0
        var made: [Call] = []
        var waiting: [Int: CheckedContinuation<JSONValue, any Error>] = [:]
        var cancelled: [Int] = []
        var calledOnMainThread = false
    }

    private let state = Mutex(State())
    /// Like a network request that completes anyway, a call can ignore its
    /// cancellation and still return when answered.
    let ignoresCancellation: Bool

    init(ignoresCancellation: Bool = false) {
        self.ignoresCancellation = ignoresCancellation
    }

    var provider: ToolProvider {
        // Checked in the closure itself, where the driver's caller decides
        // the isolation.
        { [self] name, arguments in try await call(name, arguments, onMain: pthread_main_np() != 0) }
    }

    private func call(_ name: String, _ arguments: JSONValue?, onMain: Bool) async throws -> JSONValue {
        let id = state.withLock { state in
            state.next += 1
            if onMain { state.calledOnMainThread = true }
            state.made.append(Call(id: state.next, name: name, arguments: arguments))
            return state.next
        }
        let ignores = ignoresCancellation
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<JSONValue, any Error>) in
                let early = state.withLock { state in
                    if !ignores && Task.isCancelled { return true }
                    state.waiting[id] = continuation
                    return false
                }
                if early { continuation.resume(throwing: CancellationError()) }
            }
        } onCancel: {
            let continuation = state.withLock { state in
                state.cancelled.append(id)
                if ignores { return CheckedContinuation<JSONValue, any Error>?.none }
                return state.waiting.removeValue(forKey: id)
            }
            continuation?.resume(throwing: CancellationError())
        }
    }

    /// Every call made, in order.
    var made: [Call] { state.withLock { $0.made } }
    /// Calls still waiting for an answer, in the order they were made.
    var waiting: [Call] {
        state.withLock { state in state.made.filter { state.waiting[$0.id] != nil } }
    }
    /// Whether any call ran on the main thread.
    var calledOnMainThread: Bool { state.withLock { $0.calledOnMainThread } }
    /// Calls whose task was cancelled, in order.
    var cancelled: [Int] { state.withLock { $0.cancelled } }

    func answer(_ id: Int, _ result: Result<JSONValue, any Error>) {
        let continuation = state.withLock { $0.waiting.removeValue(forKey: id) }
        continuation?.resume(with: result)
    }
    func answer(_ id: Int, _ value: JSONValue) { answer(id, .success(value)) }
    func answer(_ id: Int, throwing error: any Error) { answer(id, .failure(error)) }
}

private struct Boom: Error, CustomStringConvertible {
    var description: String { "Boom happened" }
}

private struct TimedOut: Error {}

/// Yield until the provider or the main actor has performed the named operation.
@MainActor private func eventually(_ what: String, _ condition: () -> Bool,
                                   at location: SourceLocation = #_sourceLocation) async throws {
    let limit = ContinuousClock.now + .seconds(5)
    for _ in 0..<250_000 {
        if condition() { return }
        if ContinuousClock.now >= limit { break }
        await Task.yield()
    }
    Issue.record("timed out waiting for \(what)", sourceLocation: location)
    throw TimedOut()
}

@MainActor func drawnText(_ live: LiveProgram) -> String? {
    guard case let .scalar(.string(text))? = live.root?.props.first?.value else { return nil }
    return text
}

@Suite @MainActor struct AToolProviderDriver {
    @Test func aSourceCompletionWakesTheSettledReaderDuringARawBundlePreview() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library)
        let tools = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        let raw = """
        ]]>openui:content
        s=Query("source")
        r=Query("reader",{v:s},"waiting")
        root=TextContent(r)
        ]]>openui:scripts {"count":1}
        [{"name":"reader","code":"v1"}]
        ]]>openui:end
        """
        try await live.show(response: raw, streaming: false)
        try await eventually("the source") { tools.waiting.count == 1 }
        let source = try #require(tools.waiting.first)
        #expect(source.name == "source")
        let edit = raw.replacingOccurrences(of: "v1", with: "v2")
        try await live.show(response: edit, streaming: true)
        tools.answer(source.id, .string("input"))
        try await eventually("the settled reader") { tools.waiting.first?.name == "reader" }
        let reader = try #require(tools.waiting.first)
        #expect(reader.arguments?["v"] == .string("input"))
        tools.answer(reader.id, .string("first result"))
        try await eventually("the reader result projected into the preview") { drawnText(live) == "first result" }
        #expect(live.queries.loading.isEmpty)
        try await live.show(response: edit, streaming: false)
        try await eventually("the reader with the new script revision") { tools.waiting.first?.name == "reader" }
        let revised = try #require(tools.waiting.first)
        #expect(revised.arguments == reader.arguments)
        tools.answer(revised.id, .string("second result"))
        try await eventually("the settled drawing") { drawnText(live) == "second result" }
        #expect(tools.made.map(\.name) == ["source", "reader", "reader"])
    }

    @Test func performsWantedCallsAndRedrawsWithTheirResults() async throws {
        let live = try await live("""
        $n = 1
        q = Query("read", {n: $n}, "none")
        root = TextContent("Got " + q)
        """)
        let tools = FakeTools()
        let clock = ManualClock()
        let generation = try #require(live.root?.origin?.generation)
        try live.attachToolProvider(clock: clock, tools.provider)
        try await eventually("the first call") { tools.waiting.count == 1 }
        let first = tools.waiting[0]
        #expect(live.root?.origin?.generation == generation + 1)
        #expect(first.name == "read")
        #expect(first.arguments == .object([("n", .number(1))]))
        #expect(drawnText(live) == "Got none")
        #expect(live.queries.loading == ["q"])
        #expect(live.hasToolProvider)

        tools.answer(first.id, .string("one"))
        try await eventually("the result drawn") { drawnText(live) == "Got one" }
        #expect(live.queries.loading.isEmpty)
        #expect(live.failure == nil)

        try live.set("$n", to: .number(2))
        try await eventually("the call for the new arguments") { tools.waiting.count == 1 }
        #expect(tools.waiting[0].arguments == .object([("n", .number(2))]))
        #expect(live.queries.refetching == ["q"])
        #expect(drawnText(live) == "Got one")
        tools.answer(tools.waiting[0].id, .string("two"))
        try await eventually("the new result drawn") { drawnText(live) == "Got two" }
        #expect(tools.made.count == 2)
        #expect(!tools.calledOnMainThread)
    }

    @Test func callsNothingWhileTheResponseStreams() async throws {
        let live = try await live("""
        q = Query("read", {}, "none")
        root = TextContent("Got " + q)
        """)
        live.isStreaming = true
        let tools = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        #expect(live.queries.loading.isEmpty)
        #expect(live.queryDriver?.calls.isEmpty == true)
        #expect(tools.made.isEmpty)

        let clickGate = live.suspendInteractions()
        defer { clickGate.release() }
        live.isStreaming = false
        #expect(!live.acceptsInteractions)
        try await eventually("the call once settled") { tools.waiting.count == 1 }
        tools.answer(tools.waiting[0].id, .string("live"))
        try await eventually("the result drawn") { drawnText(live) == "Got live" }
    }

    @Test func previewWritesWaitForASettledResponse() async throws {
        let source = """
        $n = 1
        q = Query("read", {n: $n}, "-")
        root = TextContent(q)
        """
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let complete = try await core.parseOneShot(source, library: library)
        let live = try LiveProgram(library: library)
        try live.show(complete)
        let tools = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        try await eventually("original call") { tools.waiting.count == 1 }
        try live.show(complete.asStreamingPreview())
        try live.set("$n", to: .number(2))
        live.isStreaming = true
        live.isStreaming = false
        #expect(live.queryDriver?.calls.count == 1)
        tools.answer(tools.waiting[0].id, .string("original"))
        try await eventually("old call delivered during preview") { live.queries.loading.isEmpty }

        live.discardPreview()
        try await eventually("arguments after discard") { tools.waiting.first?.arguments?["n"] == .number(2) }
        tools.answer(tools.waiting[0].id, .string("second"))
        try await eventually("second result") { drawnText(live) == "second" }

        try live.show(complete.asStreamingPreview())
        try live.set("$n", to: .number(3))
        #expect(live.queryDriver?.calls.isEmpty == true)
        try live.show(complete)
        try await eventually("arguments after stream finish") { tools.waiting.first?.arguments?["n"] == .number(3) }
        #expect(tools.made.count == 3)
    }

    @Test func providerCancellationEndsAMutationAsAFailure() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        m = Mutation("save")
        root = Button("Go", Action([@Run(m), @ToAssistant("done")]))
        """, events: { events.append($0) })
        let tools = FakeTools()
        try live.attachToolProvider { name, arguments in
            let result = try await tools.provider(name, arguments)
            if result == .null { withUnsafeCurrentTask { $0?.cancel() } }
            return result
        }
        for attempt in 1...3 {
            try live.run("action", of: #require(live.root), message: "go")
            try await eventually("next mutation") { tools.waiting.count == 1 }
            let task = try #require(live.queryDriver?.calls.values.first)
            let id = tools.waiting[0].id
            switch attempt {
            case 1: tools.answer(id, throwing: CancellationError())
            case 2: tools.answer(id, .null)
            default: tools.answer(id, .bool(true))
            }
            await task.value
            if attempt < 3 {
                let error = try #require(live.queries.errors.first)
                #expect(error.source == "mutation" && error.code == "tool-error")
                #expect(error.message.contains("cancelled by provider"))
                #expect(events.isEmpty)
            }
        }
        #expect(events.map(\.humanFriendlyMessage) == ["done"])
    }

    @Test func runsCallsConcurrentlyAndMapsEachFailureToItsCode() async throws {
        let live = try await live("""
        a = Query("a", {}, "-")
        b = Query("b", {}, "-")
        c = Query("c", {}, "-")
        d = Query("d", {}, "-")
        root = TextContent(a + "," + b + "," + c + "," + d)
        """)
        let tools = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        // All four are in flight before any is answered.
        try await eventually("four calls in flight") { tools.waiting.count == 4 }
        let calls = Dictionary(uniqueKeysWithValues: tools.waiting.map { ($0.name, $0.id) })
        tools.answer(calls["d"]!, .string("D"))
        try await eventually("first result before the other outcomes") { drawnText(live) == "-,-,-,D" }
        #expect(live.queries.loading.count == 3)
        tools.answer(calls["c"]!, throwing: Boom())
        tools.answer(calls["b"]!, throwing: ToolProviderError.toolReported("down"))
        tools.answer(calls["a"]!, throwing: ToolProviderError.toolNotFound(availableTools: ["x", "y"]))
        try await eventually("four outcomes") { live.queries.errors.count == 3 && live.queries.loading.isEmpty }
        #expect(drawnText(live) == "-,-,-,D")
        let errors = Dictionary(uniqueKeysWithValues: live.queries.errors.map { ($0.statementId ?? "", $0) })
        #expect(errors["a"]?.code == "tool-not-found")
        #expect(errors["a"]?.hint == "Available tools: x, y")
        #expect(errors["b"]?.code == "mcp-error")
        #expect(errors["b"]?.message.contains("down") == true)
        #expect(errors["c"]?.code == "tool-error")
        #expect(errors["c"]?.message.contains("Boom happened") == true)
        #expect(errors["c"]?.source == "query")
        #expect(errors["c"]?.toolName == .string("c"))
    }

    @Test func retriesFailedQueries() async throws {
        let live = try await live("""
        q = Query("read", {}, "none")
        root = TextContent("Got " + q)
        """)
        let tools = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        try await eventually("the call") { tools.waiting.count == 1 }
        tools.answer(tools.waiting[0].id, throwing: ToolProviderError.failed("offline"))
        try await eventually("the error") { live.queries.errors.count == 1 }
        #expect(live.queries.errors[0].message.contains("offline"))

        try live.retryFailedQueries()
        try await eventually("the retry") { tools.waiting.count == 1 }
        #expect(tools.made.count == 2)
        tools.answer(tools.waiting[0].id, .string("back"))
        try await eventually("the result") { drawnText(live) == "Got back" }
        #expect(live.queries.errors.isEmpty)
    }

    @Test func aMutationsPlanCarriesOnAndItsHostEventsReachTheHandler() async throws {
        var events: [HostEvent] = []
        var observed: [JSONValue?] = []
        weak var program: LiveProgram?
        let live = try await live("""
        $saved = false
        m = Mutation("save", {n: 1})
        root = Button("Save", Action([@Run(m), @Set($saved, true), @ToAssistant("saved"), @ToAssistant("next")]))
        """, events: {
            events.append($0)
            observed.append(program?.value(of: "$saved"))
        })
        program = live
        let tools = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        let button = try #require(live.root)
        try live.run("action", of: button, message: "Save")
        try await eventually("the mutation's call") { tools.waiting.count == 1 }
        #expect(tools.waiting[0].name == "save")
        #expect(tools.waiting[0].arguments == .object([("n", .number(1))]))
        #expect(events.isEmpty)

        tools.answer(tools.waiting[0].id, .object([("ok", .bool(true))]))
        try await eventually("the rest of the plan") { events.count == 2 }
        #expect(events.map(\.type) == ["continue_conversation", "continue_conversation"])
        #expect(events.map(\.humanFriendlyMessage) == ["saved", "next"])
        #expect(observed == [.bool(true), .bool(true)])
    }

    @Test func refreshesOnTheClockAndRearmsFromTheActualWakeTime() async throws {
        let live = try await live("""
        q = Query("read", {}, "none", 5)
        root = TextContent("Got " + q)
        """)
        let tools = FakeTools()
        let clock = ManualClock()
        try live.attachToolProvider(clock: clock, tools.provider)
        try await eventually("the first call") { tools.waiting.count == 1 }
        tools.answer(tools.waiting[0].id, .string("v1"))
        try await eventually("the first result") { drawnText(live) == "Got v1" }
        try await eventually("the timer armed") { clock.deadlines == [5000] }

        live.isStreaming = true
        clock.advance(to: 4999)
        #expect(tools.made.count == 1)
        #expect(clock.deadlines == [5000])

        // A late wake: the session hears the actual time and re-arms from it.
        clock.advance(to: 7300)
        try await eventually("the refresh call") { tools.waiting.count == 1 }
        try await eventually("the timer re-armed from the wake") { clock.deadlines == [12300] }
        tools.answer(tools.waiting[0].id, .string("v2"))
        try await eventually("the refreshed result") { drawnText(live) == "Got v2" }
        #expect(clock.deadlines == [12300])
        #expect(tools.made.count == 2)
    }

    @Test func registersIntervalsAtTheTimeOfTheDrawAndWrite() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library)
        let clock = ManualClock()
        clock.advance(to: 800)
        let tools = FakeTools()
        try live.attachToolProvider(clock: clock, minimumRefreshInterval: .milliseconds(250), tools.provider)
        let step = try await core.parseOneShot("""
        $period = 0.1
        q = Query("read", {}, "-", $period)
        root = TextContent(q)
        """, library: library)
        clock.advance(to: 9800)
        try live.show(step)
        try await eventually("deadline after the delayed draw") { clock.deadlines == [10050] }
        #expect(clock.requestedDeadlines == [10050])
        clock.advance(to: 9900)
        try live.set("$period", to: .number(1))
        try await eventually("new interval after the write") { clock.deadlines == [10900] }
        #expect(clock.requestedDeadlines == [10050, 10900])
    }

    @Test func appliesNoTimeout() async throws {
        let live = try await live("""
        q = Query("read", {}, "none")
        root = TextContent("Got " + q)
        """)
        let tools = FakeTools()
        let clock = ManualClock()
        try live.attachToolProvider(clock: clock, tools.provider)
        try await eventually("the call") { tools.waiting.count == 1 }
        clock.advance(to: 10 * 3_600_000)
        #expect(live.queryDriver?.timer == nil)
        #expect(tools.waiting.count == 1)
        #expect(tools.cancelled.isEmpty)
        #expect(live.queries.loading == ["q"])
        #expect(clock.deadlines.isEmpty)
        tools.answer(tools.waiting[0].id, .string("still wanted"))
        try await eventually("the late success drawn") { drawnText(live) == "Got still wanted" }
    }

    @Test func pausingCancelsCallsAndTheTimerAndIgnoresLateOutcomes() async throws {
        let live = try await live("""
        tick = Query("tick", {}, "-", 5)
        slow = Query("slow", {}, "-")
        root = TextContent(tick + "," + slow)
        """)
        let tools = FakeTools(ignoresCancellation: true)
        let clock = ManualClock()
        try live.attachToolProvider(clock: clock, tools.provider)
        try await eventually("two calls") { tools.waiting.count == 2 }
        let tick = try #require(tools.waiting.first { $0.name == "tick" })
        let slow = try #require(tools.waiting.first { $0.name == "slow" })
        tools.answer(tick.id, .string("T"))
        try await eventually("the tick drawn") { drawnText(live) == "T,-" }
        try await eventually("the timer armed") { clock.deadlines == [5000] }
        let pending = try #require(live.queryDriver?.calls.values.first)

        try live.pauseQueries()
        #expect(live.queriesPaused)
        try await eventually("the slow call cancelled") { tools.cancelled == [slow.id] }
        try await eventually("the timer cancelled") { clock.deadlines.isEmpty }
        tools.answer(slow.id, .string("late"))
        clock.advance(to: 6000)
        await pending.value
        #expect(drawnText(live) == "T,-")
        #expect(live.failure == nil)
        #expect(tools.made.count == 2)

        try live.resumeQueries()
        #expect(!live.queriesPaused)
        try await eventually("slow called again") { tools.waiting.contains { $0.name == "slow" } }
        try await eventually("timer resumed at the current time") { clock.deadlines == [11000] }
        let again = try #require(tools.waiting.first { $0.name == "slow" })
        tools.answer(again.id, .string("S"))
        try await eventually("the fresh result") { drawnText(live)?.hasSuffix(",S") == true }
        #expect(live.failure == nil)
    }

    @Test func pausingBeforeDispatchNeverInvokesTheProvider() async throws {
        let live = try await live("""
        q = Query("read")
        root = TextContent(q)
        """)
        let tools = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        let task = try #require(live.queryDriver?.calls.values.first)
        try live.pauseQueries()
        await task.value
        #expect(tools.made.isEmpty)
    }

    @Test func aMutationInFlightWhenQueriesPauseIsReportedOnceAsTheActionsFailure() async throws {
        // #117: the session drops the plan at the pause; the live program
        // reports it as it reports other action failures, and nothing of the
        // plan runs, even when the provider answers late.
        var events: [HostEvent] = []
        let live = try await live("""
        m = Mutation("save")
        root = Button("Go", Action([@Run(m), @ToAssistant("done")]))
        """, events: { events.append($0) })
        let tools = FakeTools(ignoresCancellation: true)
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        try live.run("action", of: #require(live.root), message: "go")
        try await eventually("the mutation's call") { tools.waiting.count == 1 }
        let pending = try #require(live.queryDriver?.calls.values.first)
        try live.pauseQueries()
        let failure = try #require(live.failure)
        #expect(failure.hasPrefix("Step 0: queries were paused"))
        tools.answer(tools.waiting[0].id, .bool(true))
        await pending.value
        try live.resumeQueries()
        #expect(live.failure == failure)
        #expect(events.isEmpty)
        #expect(tools.made.count == 1)
    }

    @Test func anEvictionSegmentReportsOneFailedStepInTheLiveProgram() async throws {
        var events: [HostEvent] = []
        let live = try await live("root = TextContent(1)", events: { events.append($0) })
        let segment = ActionOutput(events: [], failure: ActionFailureOutput(step: 1, error: .EvictedHandle),
                                   sizeLimited: false, depthLimited: false, continuation: nil)
        live.deliverQueryActions([segment], redrawWarning: nil)
        #expect(live.failure == "Step 1: continuation evicted")
        live.deliverQueryActions([], redrawWarning: nil)
        #expect(live.failure == "Step 1: continuation evicted")
        #expect(events.isEmpty)
    }

    @Test func aMutationClickedWhileQueriesArePausedFailsWithoutACall() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        m = Mutation("save")
        root = Button("Go", Action([@Run(m), @ToAssistant("done")]))
        """, events: { events.append($0) })
        let tools = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        try live.pauseQueries()
        try live.run("action", of: #require(live.root), message: "go")
        let failure = try #require(live.failure)
        #expect(failure.hasPrefix("Step 0: queries are paused"))
        try live.resumeQueries()
        #expect(live.queryDriver?.calls.isEmpty == true)
        #expect(tools.made.isEmpty)
        #expect(events.isEmpty)
    }

    @Test func detachingCancelsCallsAndIgnoresLateOutcomes() async throws {
        let live = try await live("""
        q = Query("read", {}, "none")
        root = TextContent("Got " + q)
        """)
        let tools = FakeTools(ignoresCancellation: true)
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        try await eventually("the call") { tools.waiting.count == 1 }
        let call = tools.waiting[0]
        let pending = try #require(live.queryDriver?.calls.values.first)
        try live.detachToolProvider()
        #expect(!live.hasToolProvider)
        try await eventually("the call cancelled") { tools.cancelled == [call.id] }
        #expect(drawnText(live) == "Got none")
        #expect(live.queries == QueryStatus())

        let replacement = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), replacement.provider)
        try await eventually("replacement call") { replacement.waiting.count == 1 }
        tools.answer(call.id, .string("late"))
        await pending.value
        #expect(drawnText(live) == "Got none")
        #expect(live.queries.loading == ["q"])
        replacement.answer(replacement.waiting[0].id, .string("new"))
        try await eventually("replacement result") { drawnText(live) == "Got new" }
        #expect(live.failure == nil)
    }

    @Test func goingAwayCancelsEverythingAndIsNotKeptAliveByItsTasks() async throws {
        let tools = FakeTools(ignoresCancellation: true)
        let clock = ManualClock()
        weak var gone: LiveProgram?
        let pending: Task<Void, Never>
        let session: RuntimeSession
        var events: [HostEvent] = []
        do {
            let live = try await live("""
            $saved = false
            tick = Query("tick", {}, "-", 5)
            slow = Mutation("slow", {})
            root = Button(tick, Action([@Run(slow), @Set($saved, true), @ToAssistant("late event")]))
            """, events: { events.append($0) })
            try live.attachToolProvider(clock: clock, tools.provider)
            try live.run("action", of: #require(live.root), message: "go")
            try await eventually("two calls") { tools.waiting.count == 2 }
            tools.answer(try #require(tools.waiting.first { $0.name == "tick" }).id, .string("T"))
            try await eventually("the timer") { clock.deadlines == [5000] }
            try await eventually("only the mutation pending") { live.queryDriver?.calls.count == 1 }
            pending = try #require(live.queryDriver?.calls.values.first)
            session = live.session
            gone = live
        }
        // Neither the call in flight nor the armed timer keeps it.
        #expect(gone == nil)
        let slow = try #require(tools.waiting.first { $0.name == "slow" })
        try await eventually("the call cancelled") { tools.cancelled == [slow.id] }
        try await eventually("the timer cancelled") { clock.deadlines.isEmpty }
        // A late outcome after disposal has nowhere to go.
        tools.answer(slow.id, .string("late"))
        clock.advance(to: 6000)
        await pending.value
        // Keep the native session independently observable. Even if the view
        // and its event handler are gone, the mutation must not write state.
        do {
            #expect(try session.get(name: "$saved") == "false")
        } catch OpenUiError.Disposed {
            // Explicitly disposing the session also revokes this mutation.
        }
        #expect(events.isEmpty)
        #expect(tools.made.count == 2)
    }

    @Test func aResultTheSessionRefusesBecomesTheQuerysError() async throws {
        let live = try await live("""
        q = Query("read", {}, "none")
        root = TextContent("Got " + q)
        """)
        let tools = FakeTools()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        try await eventually("the call") { tools.waiting.count == 1 }
        tools.answer(tools.waiting[0].id, .string(String(repeating: "x", count: 9 << 20)))
        try await eventually("the error") { live.queries.errors.count == 1 }
        #expect(live.queries.errors[0].code == "tool-error")
        #expect(live.queries.errors[0].message.contains("limit"), "\(live.queries.errors[0].message)")
        #expect(live.queries.loading.isEmpty)
        #expect(drawnText(live) == "Got none")
    }

    @Test func pausesWhenItsLastViewDisappearsAndResumesWhenOneAppears() async throws {
        let live = try await live("""
        q = Query("read", {}, "none")
        root = TextContent("Got " + q)
        """)
        let tools = FakeTools()
        // Drive real SwiftUI appearance callbacks in an offscreen host.
        let host = NSHostingView(rootView: AnyView(ElementView(live: live)))
        host.frame = NSRect(x: 0, y: 0, width: 160, height: 80)
        host.layoutSubtreeIfNeeded()
        try live.attachToolProvider(clock: ManualClock(), tools.provider)
        try await eventually("the call") { tools.waiting.count == 1 }
        #expect(!live.queriesPaused)

        func mount(_ content: AnyView) {
            host.rootView = content
            host.layoutSubtreeIfNeeded()
        }
        let second = NSHostingView(rootView: AnyView(ElementView(live: live)))
        second.frame = host.frame
        second.layoutSubtreeIfNeeded()
        mount(AnyView(EmptyView()))
        #expect(!live.queriesPaused)
        second.rootView = AnyView(EmptyView())
        second.layoutSubtreeIfNeeded()
        try await eventually("paused by the view") { live.queriesPaused }
        try await eventually("the call cancelled") { tools.cancelled.count == 1 }

        mount(AnyView(ElementView(live: live)))
        try await eventually("resumed by the view") { !live.queriesPaused }
        try await eventually("called again") { tools.made.count == 2 }

        // Reusing the view for another live program must balance both lifetimes.
        let replacement = try await OpenUINativeTests.live("root = TextContent(\"other\")")
        mount(AnyView(ElementView(live: replacement)))
        try await eventually("the replaced program paused") { live.queriesPaused }
        try live.pauseQueries()
        mount(AnyView(ElementView(live: live)))
        try await eventually("replacement disappeared") { replacement.queriesPaused }
        #expect(live.queriesPaused)
        try live.resumeQueries()
        #expect(!live.queriesPaused)
        try await eventually("called a third time") { tools.made.count == 3 }
    }
}
