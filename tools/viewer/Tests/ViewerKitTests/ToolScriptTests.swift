import Foundation
import OpenUINative
import SwiftUI
import Synchronization
import SwiftTestSupport
import Testing
@testable import ViewerKit

/// A monotonic clock that only moves when the test advances it.
private final class ScriptClock: Clock, Sendable {
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
    }

    private let state = Mutex(State())

    var now: Instant { state.withLock { $0.now } }
    var minimumResolution: Duration { .milliseconds(1) }

    func sleep(until deadline: Instant, tolerance: Duration?) async throws {
        let id = state.withLock { $0.next += 1; return $0.next }
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
                let immediate: Result<Void, any Error>? = state.withLock { state in
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

@Suite(NativeTestWatchdog()) @MainActor struct ScriptedToolOutcomes {
    /// Bound the wait for executor hops. Only advance(to:) moves script or refresh time.
    private func wait(_ description: String, at location: SourceLocation = #_sourceLocation,
                      until condition: () -> Bool) async throws {
        for _ in 0..<20_000 {
            if condition() { return }
            await Task.yield()
        }
        Issue.record("Waiting for \(description)", sourceLocation: location)
        throw CancellationError()
    }

    @Test func rejectsAmbiguousOutcomesAndInvalidDelays() throws {
        for json in [
            "[]", #"{"t":[]}"#, #"{"t":{}}"#, #"{"t":[0]}"#, #"{"t":[{}]}"#,
            #"{"t":[{"value":1}],"t":[{"value":2}]}"#, #"{"t":[{"value":1,"value":2}]}"#,
            #"{"t":[{"value":1,"failed":"no"}]}"#, #"{"t":[{"error":false}]}"#,
            #"{"t":[{"failed":null}]}"#, #"{"t":[{"value":3,"typo":1}]}"#,
            #"{"t":[{"value":1,"delayMs":-1}]}"#, #"{"t":[{"value":1,"delayMs":1.5}]}"#,
            #"{"t":[{"value":1,"delayMs":2147483648}]}"#, #"{"t":[{"value":1,"delayMs":"10"}]}"#
        ] {
            #expect(throws: DecodeError.self, "\(json)") { try ToolScript(json: json) }
        }
    }

    @Test func importingUTF8WithABOMReadsTheScriptAndReportsErrorsInItsJSON() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: url) }
        let bom = "\u{FEFF}"
        try Data((bom + #"{"read":[{"value":"ready"}]}"#).utf8).write(to: url)
        let script = try ToolScript(contentsOf: url)
        #expect(try await script.provider(clock: ScriptClock())("read", nil) == .string("ready"))

        try Data((bom + "{}!").utf8).write(to: url)
        let diagnostic = #expect(throws: JSONError.self) { try ToolScript(contentsOf: url) }
        #expect(diagnostic?.description == "JSON error at byte 2: trailing text")
    }

    @Test func concurrentCallsReserveOutcomesBeforeWaitingAndRepeatTheLast() async throws {
        let script = try ToolScript(json: #"""
        {"read":[{"value":{"first":[true]},"delayMs":100},{"value":null,"delayMs":20}],
         "write":[{"error":"rejected"},{"failed":"offline"}]}
        """#)
        let clock = ScriptClock()
        let provider = script.provider(clock: clock)
        let first = Task { try await provider("read", .number(1)) }
        defer { first.cancel() }
        try await wait("first delay") { clock.deadlines == [100] }
        let second = Task { try await provider("read", .number(2)) }
        defer { second.cancel() }
        try await wait("second call before the first returns") { clock.deadlines == [20, 100] }
        clock.advance(to: 20)
        #expect(try await second.value == .null)
        let third = Task { try await provider("read", nil) }
        defer { third.cancel() }
        try await wait("repeated last outcome") { clock.deadlines == [40, 100] }
        clock.advance(to: 40)
        #expect(try await third.value == .null)
        await #expect(throws: ToolProviderError.toolReported("rejected")) { try await provider("write", nil) }
        await #expect(throws: ToolProviderError.failed("offline")) { try await provider("write", nil) }
        clock.advance(to: 100)
        #expect(try await first.value == .object([("first", .array([.bool(true)]))]))
    }

    @Test func missingToolReportsTheScriptedNamesInFileOrder() async throws {
        let script = try ToolScript(json: #"{"z":[{"value":1}],"a":[{"value":2}]}"#)
        let model = CorpusModel(corpus: try Corpus.load())
        let request = ProgramRequest(title: "Missing tool", text: """
            q = Query("absent")
            root = TextContent(q)
            """, library: .corpus("bench"), toolScript: script)
        await model.open(request, clock: ScriptClock())
        let live = try #require(model.playback?.live)
        try await wait("missing tool issue") { !live.queries.errors.isEmpty }
        let issue = try #require(live.queries.errors.first)
        #expect(issue.code == "tool-not-found")
        #expect(issue.statementId == "q")
        #expect(issue.hint == "Available tools: z, a")
    }

    @Test func resumingAQueryUsesTheOutcomeAfterTheCancelledDelay() async throws {
        let model = CorpusModel(corpus: try Corpus.load())
        let clock = ScriptClock()
        let script = try ToolScript(json: #"{"read":[{"delayMs":100,"value":"first"},{"value":"second"}]}"#)
        let request = ProgramRequest(title: "Paused script", text: """
            q = Query("read")
            root = TextContent(q)
            """, library: .corpus("bench"), toolScript: script)
        await model.open(request, clock: clock)
        let live = try #require(model.playback?.live)
        defer { try? live.detachToolProvider() }
        try await wait("first outcome's delay") { clock.deadlines == [100] }

        try live.pauseQueries()
        try await wait("cancelled delay") { clock.deadlines.isEmpty }
        try live.resumeQueries()
        try await wait("second outcome after resume") { live.queries.values["q"] == .string("second") }
    }

    @Test func cancellingOverlappingCallsKeepsTheSharedToolCursor() async throws {
        let script = try ToolScript(json: #"""
        {"read":[{"delayMs":100,"value":"first"},{"delayMs":200,"value":"second"},{"value":"third"}]}
        """#)
        let clock = ScriptClock()
        let provider = script.provider(clock: clock)
        let first = Task { try await provider("read", .number(1)) }
        defer { first.cancel() }
        try await wait("first delay") { clock.deadlines == [100] }
        let second = Task { try await provider("read", .number(2)) }
        defer { second.cancel() }
        try await wait("overlapping delays") { clock.deadlines == [100, 200] }

        first.cancel()
        second.cancel()
        await #expect(throws: CancellationError.self) { try await first.value }
        await #expect(throws: CancellationError.self) { try await second.value }

        var outcome: JSONValue?
        let next = Task { outcome = try await provider("read", .number(1)) }
        defer { next.cancel() }
        try await wait("next call to return or start a delay") { outcome != nil || !clock.deadlines.isEmpty }
        #expect(outcome == .string("third"))
    }

    @Test func previewDefersTheFirstCallAndReloadRestartsTheScript() async throws {
        let model = CorpusModel(corpus: try Corpus.load())
        let clock = ScriptClock()
        let request = ProgramRequest(title: "Preview", text: """
            q = Query("read")
            root = TextContent(q)
            """, library: .corpus("bench"), toolScript: try ToolScript(json: #"{"read":[{"value":"one"},{"value":"two"}]}"#))
        await model.open(request, clock: clock)
        let playback = try #require(model.playback)
        let original = try #require(playback.live)
        try await wait("first draw") { original.queries.values["q"] == .string("one") }
        let source = ReplaySource(program: request.text, sizes: [request.text.unicodeScalars.count - 1, 1])
        await playback.load(source, resumeAt: request.text.unicodeScalars.count - 1)
        let reloaded = try #require(playback.live)
        #expect(reloaded !== original)
        #expect(reloaded.isStreaming)
        #expect(reloaded.hasToolProvider)
        #expect(reloaded.queries == QueryStatus())
        await playback.move(to: playback.chunks.count)
        try await wait("first result on the fresh runtime") { reloaded.queries.values["q"] == .string("one") }
    }

    @Test func aMutationFinishingDuringReloadKeepsItsHostEventWithTheOriginalProgram() async throws {
        let model = CorpusModel(corpus: try Corpus.load())
        let clock = ScriptClock()
        let request = ProgramRequest(title: "Publish", text: """
            save = Mutation("publish")
            root = Button("Publish", Action([@Run(save), @ToAssistant("Published")]))
            """, library: .corpus("bench"), toolScript: try ToolScript(json: #"{"publish":[{"delayMs":50,"value":true}]}"#))
        await model.open(request, clock: clock)
        let playback = try #require(model.playback)
        let original = try #require(playback.live)
        try original.run("action", of: #require(original.root), message: "Publish")
        try await wait("pending mutation") { clock.deadlines == [50] }
        await playback.load(ReplaySource(program: request.text, chunkSize: 16), resumeAt: 16)
        #expect(playback.live !== original)
        clock.advance(to: 50)
        try await wait("original mutation finished") { original.queries.values["save"]?["status"] == .string("success") }
        #expect(playback.hostEvents.isEmpty)
        #expect(playback.restoreComplete(reporting: "replacement refused"))
        #expect(playback.live === original)
        #expect(playback.hostEvents.map(\.humanFriendlyMessage) == ["Published"])
    }

    @Test func aToolReportedMutationErrorStopsItsPlanAndOffersNoQueryRetry() async throws {
        let model = CorpusModel(corpus: try Corpus.load())
        let request = ProgramRequest(title: "Declined", text: """
            $saved = false
            save = Mutation("publish")
            root = Button("Publish", Action([@Run(save), @Set($saved, true)]))
            """, library: .corpus("bench"), toolScript: try ToolScript(json: #"{"publish":[{"error":"declined"}]}"#))
        await model.open(request, clock: ScriptClock())
        let live = try #require(model.playback?.live)
        try live.run("action", of: #require(live.root), message: "Publish")
        try await wait("mutation error") { !live.queries.errors.isEmpty }
        let issue = try #require(live.queries.errors.first)
        #expect(issue.code == "mcp-error" && issue.statementId == "save")
        #expect(issue.message.contains("declined"))
        #expect(live.value(of: "$saved") == .bool(false))
        #expect(QueriesPanel(live: live).retry == nil)
    }

    /// Children as they reach ElementView(live:), including the actual action-bearing buttons.
    private func children(_ playback: Playback) -> [Element] {
        guard case let .list(items)? = playback.drawnRoot?.props.first?.value else { return [] }
        return items.compactMap { if case let .element(child) = $0 { child } else { nil } }
    }

    @Test func newsDeskLoadsRetriesRefreshesAndPublishesThroughTheDrawing() async throws {
        let clock = ScriptClock()
        let model = CorpusModel(corpus: try Corpus.load())
        await model.open(try .newsDesk(), clock: clock)
        let playback = try #require(model.playback)
        let live = try #require(playback.live)
        defer { try? live.detachToolProvider() }
        #expect(playback.step?.result.meta.errors.isEmpty == true)
        #expect(Set(live.queries.loading) == ["desk", "headline"])
        #expect(live.queries.refetching.isEmpty)
        #expect(live.queries.values["__openui_loading"] == nil)
        #expect(live.queries.values["__openui_refetching"] == nil)
        #expect(live.queries.values["__openui_errors"] == nil)
        try await wait("both first loads and the refresh interval") { clock.deadlines == [600, 900, 8000] }
        clock.advance(to: 900)
        func draws(_ text: String) -> Bool {
            children(playback).contains { element in
                guard element.typeName == "TextContent", case let .scalar(value)? = element.props.first?.value else { return false }
                return value == .string(text)
            }
        }
        try await wait("headline and desk failure") { draws("Morning edition") && !live.queries.errors.isEmpty }
        let error = try #require(live.queries.errors.first)
        #expect(error.statementId == "desk" && error.code == "tool-error")
        #expect(error.message.contains("Desk is offline"))
        let panel = QueriesPanel(live: live)
        let interaction = live.suspendInteractions()
        #expect(panel.retry == nil)
        interaction.release()
        let retry = try #require(panel.retry)
        try retry()
        try await wait("retry delay") { clock.deadlines == [1500, 8000] }
        clock.advance(to: 1500)
        try await wait("recovered desk") { live.queries.errors.isEmpty && draws("Desk connected") }
        #expect(panel.retry == nil)
        clock.advance(to: 8000)
        try await wait("scheduled refresh") { clock.deadlines == [8900, 16000] }
        #expect(live.queries.refetching == ["headline"])
        #expect(draws("Morning edition"))
        clock.advance(to: 8900)
        try await wait("refreshed drawing") { draws("Updated edition") }

        func click(_ id: String) throws {
            let button = try #require(children(playback).first { $0.statementId == id && $0.typeName == "Button" })
            try live.run("action", of: button, message: id)
        }
        try click("publish")
        try await wait("publish delay") { clock.deadlines == [9600, 16000] }
        #expect(live.queries.values["save"]?["status"] == .string("loading"))
        #expect(StoreRow.rows(of: live).first { $0.name == "$published" }?.json == "0")
        clock.advance(to: 9600)
        try await wait("refetch after publishing") { clock.deadlines == [10500, 16000] }
        #expect(live.queries.values["save"]?["status"] == .string("success"))
        #expect(StoreRow.rows(of: live).first { $0.name == "$published" }?.json == "1")
        clock.advance(to: 10500)
        try await wait("published edition") { draws("Latest edition") }
        try click("refresh")
        try await wait("manual refresh") { clock.deadlines == [11400, 16000] }
        #expect(live.queries.refetching == ["headline"])
        clock.advance(to: 11400)
        try await wait("manual refresh finished") { live.queries.loading.isEmpty }

        var rendered: [String] = []
        let drawing = ElementDrawing(chrome: { view, element, _ in
            if element.typeName == "TextContent", case let .scalar(.string(text))? = element.props.first?.value {
                rendered.append(text)
            }
            return view
        })
        let renderer = ImageRenderer(content: PlaybackDrawing(playback: playback).environment(\.elementDrawing, drawing))
        renderer.proposedSize = ProposedViewSize(width: 400, height: nil)
        #expect(renderer.nsImage != nil)
        #expect(rendered.contains("Latest edition"))
        #expect(rendered.contains("Desk connected"))
    }
}
