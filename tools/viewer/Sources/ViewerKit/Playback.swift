import Foundation
import Observation
import OpenUINative

/// Streaming parser used by playback. A factory must return a fresh session
/// each time playback rewinds. Hosts may adapt another parser or report failures.
public protocol PlaybackSession: Sendable {
    func push(_ chunks: [String]) async throws -> Step
    func finish() async throws -> Step
}

extension Session: PlaybackSession {}

/// Plays a program's chunks through a streaming parse: forward, back, to any
/// step, or at a speed. Step `k` is the result after the first `k` chunks;
/// once every chunk has arrived, the last step is the stream's end.
///
/// Going back shows exactly the result the step had going forward: recent
/// steps are kept, and older ones are rebuilt by pushing the same chunks into
/// a new session, which the core turns into the same result.
///
/// Every step shown is evaluated in a runtime session, as an app would: its
/// store carries over from step to step (what was set survives the next
/// chunk) until the program is loaded again. Until the shown step has every
/// chunk, the program counts as still streaming, so its buttons and inputs
/// are disabled as an app's are while a response arrives.
@MainActor @Observable
public final class Playback {
    public private(set) var chunks: [String] = []
    /// The program so far: every chunk that has arrived.
    public private(set) var text = ""
    /// Chunks are still arriving from a live source.
    public private(set) var isStreaming = false
    /// `textOffsets[k]` is where step `k`'s text ends, in UTF-16 units (what
    /// AppKit's text views count); `textOffsets[0]` is 0.
    public private(set) var textOffsets: [Int] = [0]
    /// The same in code points.
    private var scalarOffsets: [Int] = [0]
    /// The step shown: how many chunks the result has seen.
    public private(set) var position = 0
    public private(set) var step: Step?
    /// Where the shown step's time went; nil before the first.
    public private(set) var timings: Timings?
    /// The shown step came from the kept steps, so its timings are from when it was first made.
    public private(set) var fromCache = false
    public private(set) var isPlaying = false
    public private(set) var error: String?
    /// A parser/evaluation step was refused, as distinct from a source error
    /// followed by a successfully evaluated final step.
    public private(set) var failedToShowStep = false
    /// Chunks per second while playing.
    public var speed: Double = 30
    /// Bumped whenever a step is handed to the UI; the UI reports back when it's drawn.
    public private(set) var stamp = 0
    /// Told each step's redraw time as the UI reports it.
    @ObservationIgnored public var onDrawn: ((Duration) -> Void)?
    /// The shown step evaluated, with its store; nil before the first load.
    public private(set) var live: LiveProgram?
    /// The newest host events the program has sent since it was loaded,
    /// oldest first, at most `hostEventLimit` of them.
    public private(set) var hostEvents: [HostEvent] = []
    /// How many older host events were let go to keep within the limit. A
    /// mutation's continuation goes with its event, which ends that run.
    public private(set) var droppedHostEvents = 0
    /// How many host events are kept.
    public let hostEventLimit: Int
    /// Why the shown step couldn't be evaluated (or why there's no runtime
    /// session); the drawing then shows its parse result instead.
    public private(set) var evaluationError: String?

    private let core: Core
    private let library: Library
    /// Which props are bindings: a prompt spec for the library, if it has one.
    private let promptSpec: String?
    private let root: String?
    /// In edit mode, the program the chunks patch: each step shows it merged
    /// with the patch so far.
    public let base: String?
    private var session: (any PlaybackSession)?
    private let sessionFactory: @Sendable () async throws -> any PlaybackSession
    /// How many chunks `session` has seen.
    private var sessionPosition = 0
    private var cache: [Int: Step] = [:]
    private let cacheBudget: Int
    /// The memory the kept steps hold, estimated (see `cost(of:)`).
    private(set) var keptBytes = 0
    /// The JSON the kept steps hold, in bytes.
    var keptJSONBytes: Int { cache.values.reduce(0) { $0 + $1.json.utf8.count } }
    private var requested: Int?
    private var navigator: Task<Void, Never>?
    private var playing: Task<Void, Never>?
    private var loading: Task<Void, Never>?
    private var handedAt = ContinuousClock.now

    /// One complete presentation, including its source and actual runtime.
    /// Preview sessions never replace that runtime's store or capabilities.
    private struct Complete {
        var live: LiveProgram?
        var step: Step
        var position: Int
        var chunks: [String]
        var text: String
        var textOffsets: [Int]
        var scalarOffsets: [Int]
        var timings: Timings?
        var hostEvents: [HostEvent]
        var droppedHostEvents: Int
        var evaluationError: String?
    }
    private var complete: Complete?
    private var navigationFailed = false
    private var sourceError: String?

    /// `cacheBudget` bounds the memory the steps kept for stepping back hold,
    /// in bytes, as `cost(of:)` estimates it. With a `base`, the chunks are an
    /// edit-mode patch to it. `sessionFactory` overrides streaming sessions only;
    /// edit mode continues to merge and parse with `core`.
    public init(core: Core, library: Library, root: String?, base: String? = nil, promptSpec: String? = nil,
                cacheBudget: Int = 128 << 20, hostEventLimit: Int = 200,
                sessionFactory: (@Sendable () async throws -> any PlaybackSession)? = nil) {
        self.sessionFactory = sessionFactory ?? { try await core.session(library: library, root: root) }
        self.core = core
        self.library = library
        self.promptSpec = promptSpec
        self.root = root
        self.base = base
        self.cacheBudget = cacheBudget
        self.hostEventLimit = max(hostEventLimit, 1)
    }

    /// Replaces the program with the chunks `source` yields. While they
    /// arrive, playback follows the newest step unless it has been moved back.
    /// Once they've all arrived, it shows the step that has seen `resumeAt`
    /// code points (all of them when nil). Returns then, or when `stop()` ends
    /// a live source.
    public func load(_ source: some ChunkSource, resumeAt: Int? = nil) async {
        pause()
        let previous = loading
        let task = Task {
            // Loads run one after another: two at once would mix their chunks.
            await previous?.value
            await navigator?.value
            // Reset together, with no suspension between, so the view never
            // sees a position past the chunks.
            chunks = []
            text = ""
            textOffsets = [0]
            scalarOffsets = [0]
            position = 0
            step = nil
            timings = nil
            fromCache = false
            cache = [:]
            keptBytes = 0
            session = nil
            sessionPosition = 0
            error = nil
            sourceError = nil
            navigationFailed = false
            failedToShowStep = false
            complete?.live?.discardPreview()
            acquireGate(complete?.live)
            startLive()
            isStreaming = true
            do {
                for try await batch in source.chunks() {
                    try Task.checkCancellation()
                    let following = (requested ?? position) == chunks.count
                    append(batch)
                    if following && resumeAt == nil { enqueue(chunks.count) }
                }
            } catch is CancellationError {
                // Stopped: keep what arrived.
            } catch {
                sourceError = String(describing: error)
                self.error = sourceError
            }
            guard !navigationFailed else { return }
            isStreaming = false
            // The last step was the newest push; now it's the stream's end.
            keptBytes -= cache.removeValue(forKey: chunks.count).map(Self.cost) ?? 0
            await request(target(resumingAt: resumeAt))
        }
        loading = task
        await task.value
    }

    /// Stops a live source; what has arrived stays.
    public func stop() {
        loading?.cancel()
    }

    private func append(_ batch: [String]) {
        chunks += batch
        for chunk in batch {
            text += chunk
            textOffsets.append((textOffsets.last ?? 0) + chunk.utf16.count)
            scalarOffsets.append((scalarOffsets.last ?? 0) + chunk.unicodeScalars.count)
        }
    }

    /// How many code points the shown step has seen.
    public var received: Int { scalarOffsets[position] }

    /// The shown step is the stream's end: every chunk has arrived and been
    /// pushed. (Edit mode parses the merged program whole, with no end.)
    public var isEnd: Bool { base == nil && !isStreaming && position > 0 && position == chunks.count }

    /// The first step that has seen at least `scalars` code points.
    private func target(resumingAt scalars: Int?) -> Int {
        guard let scalars else { return chunks.count }
        return scalarOffsets.firstIndex { $0 >= scalars } ?? chunks.count
    }

    // MARK: Controls

    /// Shows step `target`; pauses playback.
    public func seek(to target: Int) {
        pause()
        enqueue(target)
    }

    /// Shows step `target`, pausing playback, and returns once it's shown.
    public func move(to target: Int) async {
        pause()
        await request(target)
    }

    public func stepForward() { seek(to: (requested ?? position) + 1) }

    public func stepBack() { seek(to: (requested ?? position) - 1) }

    /// Plays from the shown step, or from the start when at the end.
    public func play() {
        guard !isPlaying, !chunks.isEmpty else { return }
        isPlaying = true
        playing = Task {
            await navigator?.value
            if position >= chunks.count { await request(0) }
            await playLoop()
        }
    }

    public func pause() {
        isPlaying = false
        playing?.cancel()
        playing = nil
    }

    private func playLoop() async {
        var last = ContinuousClock.now
        var owed = 0.0
        while isPlaying, !Task.isCancelled, position < chunks.count {
            try? await Task.sleep(for: .milliseconds(15))
            let now = ContinuousClock.now
            owed += (now - last).inSeconds * speed
            last = now
            guard owed >= 1, isPlaying else { continue }
            let n = min(Int(owed), chunks.count - position)
            owed -= Double(n)
            await request(position + n)
        }
        if !Task.isCancelled { isPlaying = false }
    }

    // MARK: Showing steps

    /// Shows step `target` (clamped) and returns once it's shown or a later
    /// request has replaced it.
    private func request(_ target: Int) async {
        enqueue(target)
        await navigator?.value
    }

    /// Asks for step `target`, replacing any request not yet started. Steps
    /// are made one at a time, in order, so the session never sees two at once.
    private func enqueue(_ target: Int) {
        requested = min(max(target, 0), chunks.count)
        if navigator == nil {
            navigator = Task {
                while let next = requested {
                    requested = nil
                    await show(next)
                }
                navigator = nil
            }
        }
    }

    @ObservationIgnored private var interactionGates: [(live: LiveProgram, suspension: LiveProgram.InteractionSuspension)] = []

    private func acquireGate(_ program: LiveProgram?) {
        guard let program else { return }
        if !interactionGates.contains(where: { $0.live === program }) {
            interactionGates.append((program, program.suspendInteractions()))
        }
    }

    private func releaseGates(for program: LiveProgram? = nil) {
        for gate in interactionGates where program == nil || gate.live === program { gate.suspension.release() }
        interactionGates.removeAll { program == nil || $0.live === program }
    }

    private func show(_ target: Int) async {
        // Gate before the first suspension: a click delivered while navigation
        // is awaiting the parser must not act on the step being replaced.
        acquireGate(live)
        if let kept = cache[target] {
            do { try hand(kept, at: target, fromCache: true) }
            catch { failed(error) }
            return
        }
        if let base {
            // Edit mode: the merge isn't append-only, so each step parses the
            // merged program afresh, as a renderer does.
            do {
                let made = try await core.mergeAndParse(
                    base: base, patch: chunks[..<target].joined(), library: library, root: root,
                    chunks: abs(target - position), streaming: isStreaming || target < chunks.count
                )
                try hand(made, at: target, fromCache: false)
                keep(made, at: target)
            } catch {
                failed(error)
            }
            return
        }
        do {
            if session == nil || target < sessionPosition {
                session = try await sessionFactory()
                sessionPosition = 0
            }
            guard let session else { return }
            var made = try await session.push(Array(chunks[sessionPosition..<target]))
            sessionPosition = target
            if !isStreaming, target == chunks.count {
                made = try await session.finish()
            }
            try hand(made, at: target, fromCache: false)
            keep(made, at: target)
        } catch {
            failed(error)
        }
    }

    private func failed(_ error: any Error) {
        self.error = String(describing: error)
        navigationFailed = true
        failedToShowStep = true
        loading?.cancel()
        requested = nil
        releaseGates()
        live?.discardPreview()
        if let complete {
            complete.live?.discardPreview()
            if live !== complete.live {
                hostEvents = complete.hostEvents
                droppedHostEvents = complete.droppedHostEvents
            }
            live = complete.live
            evaluationError = complete.evaluationError
            chunks = complete.chunks
            text = complete.text
            textOffsets = complete.textOffsets
            scalarOffsets = complete.scalarOffsets
            position = complete.position
            step = complete.step
            timings = complete.timings
            fromCache = true
            cache = [complete.position: complete.step]
            keptBytes = Self.cost(of: complete.step)
        } else {
            position = 0
            step = nil
            timings = nil
            fromCache = false
            cache = [:]
            keptBytes = 0
        }
        isStreaming = false
        stamp += 1
        handedAt = .now
        // A failed adapter may have consumed input. Retry in a fresh parser,
        // and stop automatic playback instead of retrying the same step forever.
        session = nil
        sessionPosition = 0
        pause()
    }

    /// Restores this playback's complete source and actual runtime, reporting
    /// a failed replacement made by another playback (possibly another library).
    /// Returns false when this playback never accepted a complete program.
    @discardableResult
    public func restoreComplete(reporting message: String) -> Bool {
        guard complete != nil else { return false }
        failed(DecodeError(description: message))
        return true
    }

    /// A new runtime session, with nothing in its store.
    private func startLive() {
        hostEvents = []
        droppedHostEvents = 0
        evaluationError = nil
        do {
            live = try LiveProgram(library: library, promptSpec: promptSpec) { [weak self] event in
                self?.received(event)
            }
        } catch {
            live = nil
            evaluationError = "No runtime session: \(error)"
        }
    }

    /// Keeps the newest events; the ones let go take their continuations with them.
    private func received(_ event: HostEvent) {
        hostEvents.append(event)
        let over = hostEvents.count - hostEventLimit
        if over > 0 {
            hostEvents.removeFirst(over)
            droppedHostEvents += over
        }
        if complete?.live === live {
            complete?.hostEvents = hostEvents
            complete?.droppedHostEvents = droppedHostEvents
        }
    }

    /// Whether the drawing is the shown step evaluated; otherwise it's the
    /// step's parse result, and `evaluationError` says why.
    public var isDrawingEvaluated: Bool { live != nil && evaluationError == nil }

    /// The root of the tree the drawing shows, whose paths the selection uses:
    /// the evaluated tree's, or the parse result's when the step couldn't be
    /// evaluated.
    public var drawnRoot: Element? { isDrawingEvaluated ? live?.root : step?.result.root }

    private func hand(_ step: Step, at target: Int, fromCache: Bool) throws {
        // The playback phase also gates adapters that parse their whole text
        // on every push and return otherwise-complete one-shot Step values.
        let step = isStreaming || target < chunks.count ? step.asStreamingPreview() : step
        if let live {
            try live.show(step)
            evaluationError = nil
        }
        if step.isStreaming {
            // Keep a previous source's complete runtime gated until the
            // replacement finishes, even after its first preview is drawn.
            if let live { releaseGates(for: live) }
        } else { releaseGates() }
        // Position, parse result, timing and gate describe one accepted step.
        position = target
        self.step = step
        self.fromCache = fromCache
        timings = step.timings
        timings?.evaluate = live?.evaluation
        error = sourceError
        navigationFailed = false
        failedToShowStep = false
        if !isStreaming, target == chunks.count, !step.isStreaming {
            complete = Complete(live: live, step: step, position: target, chunks: chunks, text: text,
                textOffsets: textOffsets, scalarOffsets: scalarOffsets, timings: timings,
                hostEvents: hostEvents, droppedHostEvents: droppedHostEvents, evaluationError: evaluationError)
        }
        stamp += 1
        handedAt = .now
    }

    /// The UI drew the step with this stamp.
    public func drawn(_ stamp: Int) {
        guard stamp == self.stamp, timings?.redraw == nil else { return }
        let redraw = ContinuousClock.now - handedAt
        timings?.redraw = redraw
        onDrawn?(redraw)
    }

    /// Keeps `step` for stepping back, dropping the steps farthest from it
    /// while what's kept is over budget.
    private func keep(_ step: Step, at target: Int) {
        keptBytes -= cache.updateValue(step, forKey: target).map(Self.cost) ?? 0
        keptBytes += Self.cost(of: step)
        while keptBytes > cacheBudget, cache.count > 1,
              let farthest = cache.keys.max(by: { abs($0 - target) < abs($1 - target) }) {
            keptBytes -= cache.removeValue(forKey: farthest).map(Self.cost) ?? 0
        }
    }

    /// The memory a kept step holds, estimated from its JSON: the JSON
    /// itself, the decoded `result` (about 6 times the JSON) and the core's
    /// parse result in `parsed` (about 4 times), measured on
    /// `synthetic/dashboard-x8` in a release build. The binding reports no
    /// size for a parse result, so this is an estimate.
    static func cost(of step: Step) -> Int {
        step.json.utf8.count * 11
    }
}

extension Duration {
    public var inSeconds: Double {
        Double(components.seconds) + Double(components.attoseconds) / 1e18
    }
}
