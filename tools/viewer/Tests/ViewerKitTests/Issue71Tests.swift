import AppKit
import SwiftUI
import Observation
import OpenUINative
import Testing
import ViewerKit

private struct FinalParseFailure: Error {}

/// Holds a final parse at an explicit suspension point; no timing assumptions.
private actor FinishGate {
    private var held = false
    private var requested = false
    private var finish: CheckedContinuation<Bool, Never>?
    private var waiting: CheckedContinuation<Void, Never>?

    func arm() { held = true; requested = false }
    func decision() async -> Bool {
        guard held else { return true }
        requested = true
        waiting?.resume()
        waiting = nil
        return await withCheckedContinuation { finish = $0 }
    }
    func waitForRequest() async {
        if requested { return }
        await withCheckedContinuation { waiting = $0 }
    }
    func release(succeed: Bool) {
        held = false
        finish?.resume(returning: succeed)
        finish = nil
    }
}

private struct ControlledSession: PlaybackSession {
    let session: Session
    let gate: FinishGate
    func push(_ chunks: [String]) async throws -> Step { try await session.push(chunks) }
    func finish() async throws -> Step {
        guard await gate.decision() else { throw FinalParseFailure() }
        return try await session.finish()
    }
}

private actor WholeProgramAdapter: PlaybackSession {
    let core: Core
    let library: Library
    let gate: FinishGate
    var text = ""
    init(core: Core, library: Library, gate: FinishGate) { self.core = core; self.library = library; self.gate = gate }
    func push(_ chunks: [String]) async throws -> Step {
        text += chunks.joined()
        return try await core.parseOneShot(text, library: library)
    }
    func finish() async throws -> Step {
        guard await gate.decision() else { throw FinalParseFailure() }
        return try await core.parseOneShot(text, library: library)
    }
}

private struct HeldSource: ChunkSource {
    let stream: AsyncThrowingStream<[String], any Error>
    func chunks() -> AsyncThrowingStream<[String], any Error> { stream }
}

private struct SourceFailure: Error, CustomStringConvertible {
    var description: String { "source failed" }
}
private struct FailingSource: ChunkSource {
    func chunks() -> AsyncThrowingStream<[String], any Error> {
        AsyncThrowingStream { continuation in
            continuation.yield(["root = Button(\"Go\")"])
            continuation.finish(throwing: SourceFailure())
        }
    }
}

@Suite @MainActor struct ViewerIssue71 {
    private static let schema = #"{"$defs":{"Button":{"properties":{"label":{},"action":{}}}}}"#

    @Test func aFailedFinalParseRestoresTheCompleteProgramAndRefusesRetainedPreviews() async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let gate = FinishGate()
        let playback = Playback(core: core, library: library, root: nil, cacheBudget: 1, sessionFactory: {
            ControlledSession(session: try await core.session(library: library), gate: gate)
        })
        let source = ReplaySource(program: "$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))\nextra = 1", sizes: [49, 100])
        await playback.load(source)
        let live = try #require(playback.live)
        try live.run("action", of: try #require(live.root), message: "Go")
        #expect(live.value(of: "$n") == .number(1))
        await playback.move(to: 1)
        let preview = try #require(live.root)
        let state = live.state
        let version = live.version
        let completePosition = playback.chunks.count
        #expect(live.isStreaming)
        #expect(throws: LiveError.self) { try live.run("action", of: preview, message: "Go") }
        await gate.arm()
        let moving = Task { await playback.move(to: playback.chunks.count) }
        await gate.waitForRequest()
        #expect(throws: LiveError.self) { try live.run("action", of: preview, message: "Go") }
        #expect(live.state == state && live.version == version)
        await gate.release(succeed: false)
        await moving.value
        #expect(playback.error != nil)
        #expect(playback.position == completePosition)
        #expect(!live.isStreaming)
        #expect(throws: LiveError.self) { try live.run("action", of: preview, message: "Go") }
        #expect(live.state == state)
        // The complete session is retained, so its version is its pre-preview version.
        #expect(live.value(of: "$n") == .number(1))
        try live.run("action", of: try #require(live.root), message: "Go")
        #expect(live.value(of: "$n") == .number(2))
        await playback.move(to: playback.chunks.count)
        #expect(playback.error == nil)
        #expect(!live.isStreaming)
        try live.run("action", of: try #require(live.root), message: "Go")
        #expect(live.value(of: "$n") == .number(3))
    }

    @Test func aFailedReloadRestoresThePreviousSourceRuntimeAndClickableButton() async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let gate = FinishGate()
        let playback = Playback(core: core, library: library, root: nil, sessionFactory: {
            ControlledSession(session: try await core.session(library: library), gate: gate)
        })
        let originalSource = "$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))"
        await playback.load(ReplaySource(program: originalSource, chunkSize: 100))
        let original = try #require(playback.live)
        let button = try #require(original.root)
        try original.run("action", of: button, message: "Go")
        let state = original.state
        let version = original.version
        await gate.arm()
        let (stream, continuation) = AsyncThrowingStream<[String], any Error>.makeStream()
        let loading = Task { await playback.load(HeldSource(stream: stream)) }
        await withCheckedContinuation { (started: CheckedContinuation<Void, Never>) in
            withObservationTracking { _ = playback.isStreaming } onChange: { started.resume() }
        }
        await withCheckedContinuation { (drawn: CheckedContinuation<Void, Never>) in
            withObservationTracking { _ = playback.step } onChange: { drawn.resume() }
            continuation.yield(["$new = 9\nroot = Button(\"Wrong\", @Set($n, 99))"])
        }
        #expect(playback.step?.isStreaming == true)
        #expect(throws: LiveError.self) { try original.run("action", of: button, message: "Go") }
        #expect(original.state == state && original.version == version)
        continuation.finish()
        await gate.waitForRequest()
        #expect(throws: LiveError.self) { try original.run("action", of: button, message: "Go") }
        #expect(original.state == state && original.version == version)
        await gate.release(succeed: false)
        await loading.value
        #expect(playback.error != nil)
        #expect(playback.live === original)
        #expect(playback.text == originalSource)
        #expect(playback.position == playback.chunks.count)
        #expect(original.state == state && original.version == version)
        try original.run("action", of: button, message: "Go")
        #expect(original.value(of: "$n") == .number(2))
        #expect(original.value(of: "$new") == nil)
    }

    @Test func aRejectedReplacementRestoresAnotherPlaybacksCompleteRuntime() async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let previous = Playback(core: core, library: library, root: nil)
        let source = "$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))"
        await previous.load(ReplaySource(program: source, chunkSize: 100))
        let live = try #require(previous.live)
        let button = try #require(live.root)
        try live.run("action", of: button, message: "Go")
        let state = live.state
        let version = live.version
        let other = try await core.library(schema: #"{"$defs":{"TextContent":{"properties":{"text":{}}}}}"#)
        let gate = FinishGate()
        await gate.arm()
        let replacement = Playback(core: core, library: other, root: nil, sessionFactory: {
            ControlledSession(session: try await core.session(library: other), gate: gate)
        })
        let loading = Task { await replacement.load(ReplaySource(program: "root = TextContent(\"rejected\")", chunkSize: 8)) }
        await gate.waitForRequest()
        await gate.release(succeed: false)
        await loading.value
        #expect(replacement.failedToShowStep)
        let message = try #require(replacement.error)
        #expect(previous.restoreComplete(reporting: message))
        #expect(previous.error == message)
        #expect(previous.live === live)
        #expect(previous.text == source)
        #expect(live.state == state && live.version == version)
        try live.run("action", of: button, message: "Go")
        #expect(live.value(of: "$n") == .number(2))
    }

    @Test func wholeProgramParserAdaptersCannotMakeIntermediateStepsClickable() async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let gate = FinishGate()
        let playback = Playback(core: core, library: library, root: nil, cacheBudget: 1, sessionFactory: {
            WholeProgramAdapter(core: core, library: library, gate: gate)
        })
        await playback.load(ReplaySource(program: "$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))\nextra = 1", sizes: [49, 100]))
        let live = try #require(playback.live)
        let original = try #require(live.root)
        try live.run("action", of: original, message: "Go")
        await playback.move(to: 1)
        let preview = try #require(live.root)
        #expect(playback.step?.isStreaming == true)
        await gate.arm()
        let moving = Task { await playback.move(to: playback.chunks.count) }
        await gate.waitForRequest()
        #expect(throws: LiveError.self) { try live.run("action", of: preview, message: "Go") }
        await gate.release(succeed: false)
        await moving.value
        #expect(throws: LiveError.self) { try live.run("action", of: preview, message: "Go") }
        #expect(live.value(of: "$n") == .number(1))
        try live.run("action", of: original, message: "Go")
        #expect(live.value(of: "$n") == .number(2))
    }

    @Test(arguments: [false, true], [false, true])
    func finalParsingRestoresOnlyTheViewersGate(_ requested: Bool, _ succeeds: Bool) async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let gate = FinishGate()
        let playback = Playback(core: core, library: library, root: nil, cacheBudget: 1, sessionFactory: {
            ControlledSession(session: try await core.session(library: library), gate: gate)
        })
        await playback.load(ReplaySource(program: "$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))\nextra = 1", sizes: [49, 100]))
        let live = try #require(playback.live)
        let complete = try #require(live.root)
        let state = live.state
        let version = live.version
        live.isStreaming = requested
        await playback.move(to: 1)
        #expect(live.isStreamingRequested == requested)
        await gate.arm()
        let moving = Task { await playback.move(to: playback.chunks.count) }
        await gate.waitForRequest()
        #expect(throws: LiveError.self) { try live.run("action", of: complete, message: "Go") }
        await gate.release(succeed: succeeds)
        await moving.value
        #expect(live.state == state && live.version == version)
        #expect(live.isStreamingRequested == requested)
        #expect(live.isStreaming == requested)
        #expect(live.acceptsInteractions == !requested)
        let button = try #require(live.root)
        if !succeeds { #expect(live.isCurrent(complete)) }
        if requested {
            #expect(throws: LiveError.self) { try live.run("action", of: button, message: "Go") }
            #expect(live.state == state && live.version == version)
        } else {
            try live.run("action", of: button, message: "Go")
            #expect(live.value(of: "$n") == .number(1))
        }
    }

    @Test(arguments: [false, true])
    func changingTheAppsRequestWhileFinalParsingIsSuspendedSurvivesItsFailure(_ requested: Bool) async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let gate = FinishGate()
        let playback = Playback(core: core, library: library, root: nil, cacheBudget: 1, sessionFactory: {
            ControlledSession(session: try await core.session(library: library), gate: gate)
        })
        await playback.load(ReplaySource(program: "$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))\nextra = 1", sizes: [49, 100]))
        let live = try #require(playback.live)
        let original = try #require(live.root)
        live.isStreaming = requested
        await playback.move(to: 1)
        await gate.arm()
        let moving = Task { await playback.move(to: playback.chunks.count) }
        await gate.waitForRequest()
        #expect(live.isStreamingRequested == requested)
        live.isStreaming = !requested
        #expect(live.isStreaming)
        await gate.release(succeed: false)
        await moving.value
        #expect(live.isStreamingRequested == !requested)
        #expect(live.isStreaming == !requested)
        #expect(live.isCurrent(original))
        if !requested { #expect(throws: LiveError.self) { try live.run("action", of: original, message: "Go") } }
        else {
            try live.run("action", of: original, message: "Go")
            #expect(live.value(of: "$n") == .number(1))
        }
    }

    @Test func aSourceFailureRemainsReportedAfterSuccessfulFinalEvaluation() async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let playback = Playback(core: core, library: library, root: nil)
        await playback.load(FailingSource())
        #expect(playback.error == "source failed")
        #expect(!playback.failedToShowStep)
        #expect(playback.drawnRoot?.typeName == "Button")
        #expect(playback.isEnd)
        #expect(playback.live?.isStreaming == false)
    }

    @Test func automaticPlaybackStopsAfterAFailedFinalStep() async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let gate = FinishGate()
        let playback = Playback(core: core, library: library, root: nil, cacheBudget: 1, sessionFactory: {
            ControlledSession(session: try await core.session(library: library), gate: gate)
        })
        await playback.load(ReplaySource(program: "root = Button(\"Go\")\nextra = 1", sizes: [20, 100]))
        await playback.move(to: 1)
        await gate.arm()
        playback.speed = 10000
        playback.play()
        await gate.waitForRequest()
        await withCheckedContinuation { (paused: CheckedContinuation<Void, Never>) in
            withObservationTracking { _ = playback.isPlaying } onChange: { paused.resume() }
            Task { await gate.release(succeed: false) }
        }
        #expect(!playback.isPlaying)
        #expect(playback.error != nil)
        #expect(playback.position == playback.chunks.count)
        #expect(playback.live?.isStreaming == false)
        playback.pause()
    }
}
