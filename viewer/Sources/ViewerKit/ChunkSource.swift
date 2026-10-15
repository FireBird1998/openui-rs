/// Where a program's chunks come from. Playback, the drawing and the
/// inspector only ever see chunks, so they can't tell a replay from a
/// recording or a live model.
public protocol ChunkSource: Sendable {
    /// The chunks in order, in batches: a replay yields them all in one batch,
    /// a live stream a few at a time as they arrive.
    func chunks() -> AsyncThrowingStream<[String], any Error>
}

/// A program replayed in chunks of a fixed number of code points, the way the
/// corpus chunkings cut it.
public struct ReplaySource: ChunkSource {
    public var program: String
    public var chunkSize: Int

    public init(program: String, chunkSize: Int) {
        precondition(chunkSize > 0, "chunks need at least one code point")
        self.program = program
        self.chunkSize = chunkSize
    }

    public func chunks() -> AsyncThrowingStream<[String], any Error> {
        let all = ReplaySource.split(program, size: chunkSize)
        return AsyncThrowingStream { continuation in
            if !all.isEmpty { continuation.yield(all) }
            continuation.finish()
        }
    }

    /// Consecutive pieces of `size` code points (Unicode scalars), the last
    /// one shorter.
    public static func split(_ text: String, size: Int) -> [String] {
        let scalars = Array(text.unicodeScalars)
        return stride(from: 0, to: scalars.count, by: size).map { start in
            var piece = String.UnicodeScalarView()
            piece.append(contentsOf: scalars[start..<min(start + size, scalars.count)])
            return String(piece)
        }
    }
}
