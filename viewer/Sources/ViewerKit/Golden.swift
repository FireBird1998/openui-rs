import Foundation

/// How a program is cut into chunks: a fixed number of code points, or one
/// of the corpus's seeded random chunkings with the sizes its golden records.
public enum Chunking: Hashable, Sendable {
    case fixed(Int)
    case recorded(name: String, sizes: [Int])

    /// The corpus's fixed chunkings, then larger ones.
    public static let fixedSizes = [1, 2, 3, 4, 7, 16, 64, 256]

    /// The chunking's name in the corpus goldens.
    public var goldenName: String {
        switch self {
        case let .fixed(size): String(size)
        case let .recorded(name, _): name
        }
    }

    public var label: String {
        switch self {
        case let .fixed(size): size == 1 ? "1 character" : "\(size) characters"
        case let .recorded(name, _): "corpus \(name)"
        }
    }

    public func source(for program: String) -> ReplaySource {
        switch self {
        case let .fixed(size): ReplaySource(program: program, chunkSize: size)
        case let .recorded(_, sizes): ReplaySource(program: program, sizes: sizes)
        }
    }
}

/// The reference's fingerprints of every streaming step of one case, per
/// chunking, from `conformance/golden/<case>.steps.json`. A golden's last step
/// is the stream's end, which `ends` holds apart from the pushes.
public struct GoldenSteps: Sendable {
    /// Chunking name to one fingerprint per push.
    public var steps: [String: [String]]
    /// Chunking name to the fingerprint once the stream has ended.
    public var ends: [String: String]
    /// Chunk sizes of the seeded random and recorded chunkings.
    public var sizes: [String: [Int]]

    public init(steps: [String: [String]], ends: [String: String] = [:], sizes: [String: [Int]] = [:]) {
        self.steps = steps
        self.ends = ends
        self.sizes = sizes
    }

    public static func load(caseName: String, in corpus: URL) throws -> GoldenSteps {
        let url = corpus.appendingPathComponent("golden/\(caseName).steps.json")
        let json = try JSONValue.parse(String(contentsOf: url, encoding: .utf8))
        var golden = GoldenSteps(steps: [:])
        guard case let .object(chunkings) = json else { return golden }
        for (name, value) in chunkings {
            let all = value["steps"]?.array?.compactMap(\.string) ?? []
            golden.steps[name] = Array(all.dropLast())
            golden.ends[name] = all.last
            if let sizes = value["sizes"]?.array {
                golden.sizes[name] = sizes.compactMap { $0.number.map(Int.init) }
            }
        }
        return golden
    }

    /// The corpus's random chunkings for this case, which the viewer can replay.
    public var recordedChunkings: [Chunking] {
        sizes.keys.sorted().compactMap { name in sizes[name].map { .recorded(name: name, sizes: $0) } }
    }

    /// Whether `json`, the result after `step` pushes under `chunking`, is the
    /// reference's. With `ended`, the last step is the stream's end, and is
    /// checked against the golden's end.
    public func verdict(step: Int, json: String, chunking: Chunking, ended: Bool = false) -> GoldenVerdict {
        guard let steps = steps[chunking.goldenName] else { return .noGolden }
        guard step > 0 else { return .nothingPushed }
        guard step <= steps.count else { return .noGolden }
        let reference = ended && step == steps.count ? ends[chunking.goldenName] : steps[step - 1]
        guard let reference else { return .noGolden }
        let mine = fingerprint(json)
        return mine == reference ? .matches : .differs(reference: reference, viewer: mine)
    }
}

public enum GoldenVerdict: Equatable, Sendable {
    case matches
    case differs(reference: String, viewer: String)
    case nothingPushed
    case noGolden
}

/// FNV-1a (32-bit) over UTF-8 bytes, as 8 hex digits: the goldens' fingerprint.
public func fingerprint(_ json: String) -> String {
    var hash: UInt32 = 0x811C_9DC5
    for byte in json.utf8 {
        hash ^= UInt32(byte)
        hash = hash &* 0x0100_0193
    }
    return String(format: "%08x", hash)
}
