import Foundation
import Observation

/// Plays a program's chunks through a streaming parse: forward, back, to any
/// step, or at a speed. Step `k` is the result after the first `k` chunks.
///
/// Going back shows exactly the result the step had going forward: recent
/// steps are kept, and older ones are rebuilt by pushing the same chunks into
/// a new session, which the core turns into the same result.
@MainActor @Observable
public final class Playback {
    public private(set) var chunks: [String] = []
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
    /// Chunks per second while playing.
    public var speed: Double = 30
    /// Bumped whenever a step is handed to the UI; the UI reports back when it's drawn.
    public private(set) var stamp = 0
    /// Told each step's redraw time as the UI reports it.
    @ObservationIgnored public var onDrawn: ((Duration) -> Void)?

    private let core: Core
    private let library: Library
    private let root: String?
    private var session: Session?
    /// How many chunks `session` has seen.
    private var sessionPosition = 0
    private var cache: [Int: Step] = [:]
    private let cacheBudget: Int
    private var requested: Int?
    private var navigator: Task<Void, Never>?
    private var playing: Task<Void, Never>?
    private var loading: Task<Void, Never>?
    private var handedAt = ContinuousClock.now

    /// `cacheBudget` bounds the JSON kept for stepping back, in bytes.
    public init(core: Core, library: Library, root: String?, cacheBudget: Int = 32 << 20) {
        self.core = core
        self.library = library
        self.root = root
        self.cacheBudget = cacheBudget
    }

    /// Replaces the program with the chunks `source` yields, and shows the
    /// step that has seen `resumeAt` code points (all of them when nil).
    /// Returns once every chunk has arrived and that step is shown.
    public func load(_ source: some ChunkSource, resumeAt: Int? = nil) async {
        pause()
        loading?.cancel()
        let task = Task {
            await navigator?.value
            chunks = []
            textOffsets = [0]
            scalarOffsets = [0]
            cache = [:]
            session = nil
            sessionPosition = 0
            error = nil
            do {
                for try await batch in source.chunks() {
                    chunks += batch
                    for chunk in batch {
                        textOffsets.append(textOffsets.last! + chunk.utf16.count)
                        scalarOffsets.append(scalarOffsets.last! + chunk.unicodeScalars.count)
                    }
                }
            } catch {
                self.error = String(describing: error)
            }
            await request(target(resumingAt: resumeAt))
        }
        loading = task
        await task.value
    }

    /// How many code points the shown step has seen.
    public var received: Int { scalarOffsets[position] }

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

    private func show(_ target: Int) async {
        if let kept = cache[target] {
            hand(kept, at: target, fromCache: true)
            return
        }
        do {
            if session == nil || target < sessionPosition {
                session = try await core.session(library: library, root: root)
                sessionPosition = 0
            }
            guard let session else { return }
            let made = try await session.push(Array(chunks[sessionPosition..<target]))
            sessionPosition = target
            keep(made, at: target)
            hand(made, at: target, fromCache: false)
        } catch {
            self.error = String(describing: error)
        }
    }

    private func hand(_ step: Step, at target: Int, fromCache: Bool) {
        position = target
        self.step = step
        self.fromCache = fromCache
        timings = step.timings
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
    /// while the kept JSON is over budget.
    private func keep(_ step: Step, at target: Int) {
        cache[target] = step
        var size = cache.values.reduce(0) { $0 + $1.json.utf8.count }
        while size > cacheBudget, cache.count > 1,
              let farthest = cache.keys.max(by: { abs($0 - target) < abs($1 - target) }) {
            size -= cache.removeValue(forKey: farthest)?.json.utf8.count ?? 0
        }
    }
}

extension Duration {
    public var inSeconds: Double {
        Double(components.seconds) + Double(components.attoseconds) / 1e18
    }
}
