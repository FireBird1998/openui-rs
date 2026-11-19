import OpenUINative

/// Replays the raw prefix, since the winning content and scripts boundary can move.
actor ResponsePlaybackSession: PlaybackSession {
    private let parser: Session
    private var text = ""

    init(parser: Session) {
        self.parser = parser
    }

    func push(_ chunks: [String]) async throws -> Step {
        text += chunks.joined()
        var step = try await parser.response(text, streaming: true)
        step.timings.chunks = chunks.count
        return step
    }

    func finish() async throws -> Step {
        var step = try await parser.response(text, streaming: false)
        step.timings.chunks = 0
        return step
    }
}
