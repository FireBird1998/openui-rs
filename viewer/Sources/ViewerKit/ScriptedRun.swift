import AppKit

/// Checks playback in the real window with nobody at the screen:
///
///   VIEWER_PLAY=<case> [VIEWER_CHUNK=1] [VIEWER_SECONDS=6] Viewer.app/Contents/MacOS/Viewer
///
/// opens the case, plays it as fast as possible, jumps back mid-play, plays
/// on, pauses, prints how the UI kept up, and quits.
@MainActor
enum ScriptedRun {
    struct Request {
        var caseName: String
        var chunkSize: Int
        var seconds: Double
    }

    static var request: Request? {
        let env = ProcessInfo.processInfo.environment
        guard let name = env["VIEWER_PLAY"] else { return nil }
        return Request(
            caseName: name,
            chunkSize: env["VIEWER_CHUNK"].flatMap(Int.init) ?? 1,
            seconds: env["VIEWER_SECONDS"].flatMap(Double.init) ?? 6
        )
    }

    static func run(_ request: Request, model: CorpusModel) async {
        model.chunking = .fixed(request.chunkSize)
        model.selected = request.caseName
        // The selection opens the case; wait until it shows its last step.
        var waited = 0
        while model.playback.map({ $0.chunks.isEmpty || $0.position < $0.chunks.count }) ?? true {
            try? await Task.sleep(for: .milliseconds(20))
            waited += 1
            if waited > 1500 { return finish("scripted run: \(request.caseName) never opened (\(model.failure ?? "no error"))") }
        }
        guard let playback = model.playback else { return }

        var redraws: [Duration] = []
        var updates: [Timings] = []
        playback.onDrawn = { redraw in
            redraws.append(redraw)
            if let t = playback.timings, !playback.fromCache { updates.append(t) }
        }
        let heartbeat = Heartbeat()
        let started = ContinuousClock.now
        let stampAtStart = playback.stamp

        playback.speed = 10000
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

        let sorted = redraws.sorted()
        let medianRedraw = sorted.isEmpty ? Duration.zero : sorted[sorted.count / 2]
        func median(_ part: KeyPath<Timings, Duration>) -> String {
            let values = updates.map { $0[keyPath: part] }.sorted()
            return TimingsLine.ms(values.isEmpty ? .zero : values[values.count / 2])
        }
        let chunksPerUpdate = updates.isEmpty ? 0 : updates.map(\.chunks).reduce(0, +) / updates.count
        let elapsed = ContinuousClock.now - started
        finish("""
        scripted run: \(request.caseName), \(request.chunkSize) code point(s) per chunk, \
        \(playback.chunks.count) chunks, \(String(format: "%.1f", elapsed.inSeconds)) s
          updates drawn: \(playback.stamp - stampAtStart); reached chunk \(playback.position)
          per update (median, about \(chunksPerUpdate) chunks each): parse \(median(\.parse)), \
        crossing \(median(\.crossing)), decode \(median(\.decode))
          redraw: median \(TimingsLine.ms(medianRedraw)), worst \(TimingsLine.ms(sorted.last ?? .zero))
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
