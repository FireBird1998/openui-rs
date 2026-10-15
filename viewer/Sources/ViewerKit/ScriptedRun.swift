import AppKit

/// Checks playback in the real window with nobody at the screen:
///
///   VIEWER_PLAY=<case> [VIEWER_CHUNK=1] [VIEWER_SECONDS=6] [VIEWER_SPEED=<chunks/s>] Viewer.app/Contents/MacOS/Viewer
///
/// opens the case (or, for `gallery`, the component gallery; for an absolute
/// path, that program file against `VIEWER_LIBRARY`, default `bench`), plays it
/// (as fast as possible unless `VIEWER_SPEED` says otherwise), jumps back
/// mid-play, plays on, pauses, prints how the UI kept up, and quits.
@MainActor
enum ScriptedRun {
    struct Request {
        var caseName: String
        var chunkSize: Int
        var seconds: Double
        /// Chunks per second; as fast as possible by default.
        var speed: Double
    }

    static var request: Request? {
        let env = ProcessInfo.processInfo.environment
        guard let name = env["VIEWER_PLAY"] else { return nil }
        return Request(
            caseName: name,
            chunkSize: env["VIEWER_CHUNK"].flatMap(Int.init) ?? 1,
            seconds: env["VIEWER_SECONDS"].flatMap(Double.init) ?? 6,
            speed: env["VIEWER_SPEED"].flatMap(Double.init) ?? 10000
        )
    }

    static func run(_ request: Request, model: CorpusModel) async {
        model.chunking = .fixed(request.chunkSize)
        if request.caseName == "gallery" {
            await model.open(Gallery.request)
        } else if request.caseName.hasPrefix("/") {
            let library = ProcessInfo.processInfo.environment["VIEWER_LIBRARY"] ?? "bench"
            guard let text = try? String(contentsOfFile: request.caseName, encoding: .utf8) else {
                return finish("scripted run: can't read \(request.caseName)")
            }
            await model.open(ProgramRequest(title: request.caseName, text: text, library: .corpus(library), root: nil))
        } else {
            model.selected = request.caseName
        }
        // The selection opens the case; wait until it shows its last step.
        var waited = 0
        while model.playback.map({ $0.chunks.isEmpty || $0.position < $0.chunks.count }) ?? true {
            try? await Task.sleep(for: .milliseconds(20))
            waited += 1
            if waited > 1500 { return finish("scripted run: \(request.caseName) never opened (\(model.failure ?? "no error"))") }
        }
        guard let playback = model.playback else { return }

        // Every update made (not taken from the kept steps), once it's drawn.
        var updates: [Timings] = []
        playback.onDrawn = { _ in
            if let t = playback.timings, !playback.fromCache { updates.append(t) }
        }
        let heartbeat = Heartbeat()
        let started = ContinuousClock.now
        let stampAtStart = playback.stamp

        playback.speed = request.speed
        playback.play()
        try? await Task.sleep(for: .seconds(request.seconds / 2))
        let back = playback.position / 2
        let seekStarted = ContinuousClock.now
        await playback.move(to: back)
        let seekTook = ContinuousClock.now - seekStarted
        playback.play()
        try? await Task.sleep(for: .seconds(request.seconds / 2))
        playback.pause()
        let stampAtPause = playback.stamp
        try? await Task.sleep(for: .milliseconds(500))
        let afterPause = playback.stamp - stampAtPause
        heartbeat.stop()

        func part(_ name: String, _ values: [Duration]) -> String {
            let sorted = values.sorted()
            let median = sorted.isEmpty ? Duration.zero : sorted[sorted.count / 2]
            return "    \(name.padding(toLength: 9, withPad: " ", startingAt: 0))"
                + "\(TimingsLine.ms(median)) / \(TimingsLine.ms(sorted.last ?? .zero))"
        }
        let chunksPerUpdate = updates.isEmpty ? 0 : updates.map(\.chunks).reduce(0, +) / updates.count
        let elapsed = ContinuousClock.now - started
        finish("""
        scripted run: \(request.caseName), \(request.chunkSize) code point(s) per chunk, \
        \(playback.chunks.count) chunks, \(String(format: "%.1f", elapsed.inSeconds)) s
          updates drawn: \(playback.stamp - stampAtStart); reached chunk \(playback.position)
          per update, median / worst (\(updates.count) updates, about \(chunksPerUpdate) chunks each):
        \(part("parse", updates.map(\.parse)))
        \(part("crossing", updates.map(\.crossing)))
        \(part("decode", updates.map(\.decode)))
        \(part("redraw", updates.compactMap(\.redraw)))
          main thread's longest stall: \(TimingsLine.ms(heartbeat.longestStall))
          jump back to chunk \(back) mid-play: \(TimingsLine.ms(seekTook))
          updates after pause: \(afterPause)
        """)
    }

    private static func finish(_ report: String) {
        print(report)
        fflush(stdout)
        NSApp.terminate(nil)
    }
}

/// Ticks on the main run loop and records the longest gap between ticks: how
/// long a click or key press could have waited.
@MainActor
private final class Heartbeat {
    private var timer: Timer?
    private var last = ContinuousClock.now
    private(set) var longestStall = Duration.zero

    init() {
        let timer = Timer(timeInterval: 0.005, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    private func tick() {
        let now = ContinuousClock.now
        longestStall = max(longestStall, now - last - .milliseconds(5))
        last = now
    }

    func stop() {
        timer?.invalidate()
    }
}
