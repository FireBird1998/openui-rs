/// Where a program's chunks come from. Playback, the drawing and the
/// inspector only ever see chunks, so they can't tell a replay from a
/// recording or a live model.
public protocol ChunkSource: Sendable {
    /// The chunks in order, in batches: a replay yields them all in one batch,
    /// a live stream a few at a time as they arrive.
    func chunks() -> AsyncThrowingStream<[String], any Error>
}

/// A program replayed in chunks of a given number of code points each, the
/// way the corpus chunkings cut it.
public struct ReplaySource: ChunkSource {
    public var program: String
    /// Code points per chunk; the last size repeats until the program ends.
    public var sizes: [Int]

    /// Chunks of `chunkSize` code points, the last one shorter.
    public init(program: String, chunkSize: Int) {
        self.init(program: program, sizes: [chunkSize])
    }

    /// Chunks of the given sizes in turn, as a seeded random chunking records them.
    public init(program: String, sizes: [Int]) {
        precondition(!sizes.isEmpty && sizes.allSatisfy { $0 > 0 }, "chunks need at least one code point")
        self.program = program
        self.sizes = sizes
    }

    public func chunks() -> AsyncThrowingStream<[String], any Error> {
        let all = ReplaySource.split(program, sizes: sizes)
        return AsyncThrowingStream { continuation in
            if !all.isEmpty { continuation.yield(all) }
            continuation.finish()
        }
    }

    /// Consecutive pieces of `size` code points (Unicode scalars), the last
    /// one shorter.
    public static func split(_ text: String, size: Int) -> [String] {
        split(text, sizes: [size])
    }

    /// Consecutive pieces of the given sizes in code points, the last size
    /// repeating until the text ends, and the last piece possibly shorter.
    public static func split(_ text: String, sizes: [Int]) -> [String] {
        let scalars = Array(text.unicodeScalars)
        var pieces: [String] = []
        var start = 0
        while start < scalars.count {
            let size = sizes[min(pieces.count, sizes.count - 1)]
            var piece = String.UnicodeScalarView()
            piece.append(contentsOf: scalars[start..<min(start + size, scalars.count)])
            pieces.append(String(piece))
            start += size
        }
        return pieces
    }
}
