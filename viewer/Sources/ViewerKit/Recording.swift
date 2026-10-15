import AppKit
import Foundation

/// A live stream saved to replay later, or to become a corpus case with a
/// recorded chunking. One JSON file per recording:
///
/// ```json
/// {
///   "format": "openui-recording", "version": 1,
///   "createdAt": "2026-10-15T18:03:12Z",
///   "request": "a dashboard for a weather station",
///   "model": "claude-opus-5-5", "servedBy": "claude-opus-5-5", "effort": "medium",
///   "library": "bench",
///   "stopReason": "end_turn",
///   "inputTokens": 2146, "outputTokens": 2950,
///   "chunks": [{"t": 1.84, "text": "root = "}, {"t": 1.86, "text": "Stack(["}]
/// }
/// ```
///
/// A follow-up in edit mode records the program it patched in `patchOf`, and
/// its chunks are the patch: each step is `patchOf` merged with the patch so
/// far. A fresh request has no `patchOf`.
///
/// `t` is seconds from sending the request to the delta's arrival; each
/// chunk's text is a text delta exactly as received. `model` is what was
/// asked for and `servedBy` what answered (they differ after a fallback).
/// `stopReason` is left out when the stream was stopped or failed, as is any
/// other field without a value. A recording never holds the API key or any
/// request header.
public struct Recording: Codable, Equatable, Sendable {
    public static let formatName = "openui-recording"

    public struct Chunk: Codable, Equatable, Sendable {
        /// Seconds from sending the request to this delta's arrival.
        public var t: Double
        public var text: String
    }

    public var format = Recording.formatName
    public var version = 1
    public var createdAt: Date
    public var request: String
    public var model: String
    public var servedBy: String?
    public var effort: String
    public var library: String
    public var stopReason: String?
    public var inputTokens: Int
    public var outputTokens: Int
    public var chunks: [Chunk]
    /// For a follow-up, the program the chunks patch.
    public var patchOf: String?

    /// The whole program, or for a follow-up the whole patch, as it arrived.
    public var text: String { chunks.map(\.text).joined() }

    /// The chunks' sizes in code points: replaying with these sizes gives
    /// back exactly the recorded chunks.
    public var sizes: [Int] { chunks.map { $0.text.unicodeScalars.count } }

    /// The chunking that cuts the text where the deltas arrived.
    public var chunking: Chunking { .recorded(name: "as received", sizes: sizes) }

    public func encoded() throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        encoder.dateEncodingStrategy = .iso8601
        return try encoder.encode(self)
    }

    public static func decode(_ data: Data) throws -> Recording {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let recording = try decoder.decode(Recording.self, from: data)
        guard recording.format == formatName else {
            throw DecodeError(description: "not an OpenUI recording (format \(recording.format))")
        }
        return recording
    }
}

extension Recording {
    /// A recording of what a live request received.
    @MainActor
    init(status: LiveStatus, request: String, effort: String, library: String, createdAt: Date, patchOf: String? = nil) {
        var stopReason: String?
        if case let .finished(reason) = status.phase, reason != "end of stream" { stopReason = reason }
        self.init(
            createdAt: createdAt,
            request: request,
            model: status.model.id,
            servedBy: status.servedBy,
            effort: effort,
            library: library,
            stopReason: stopReason,
            inputTokens: status.inputTokens,
            outputTokens: status.outputTokens,
            chunks: status.received,
            patchOf: patchOf
        )
    }
}

/// Replays a recording's chunks: all at once, or with the timing they arrived with.
public struct RecordingSource: ChunkSource {
    public let recording: Recording
    /// Wait between chunks as the recording did, instead of yielding them all at once.
    public let timed: Bool

    public init(recording: Recording, timed: Bool) {
        self.recording = recording
        self.timed = timed
    }

    public func chunks() -> AsyncThrowingStream<[String], any Error> {
        let chunks = recording.chunks
        guard timed else {
            return AsyncThrowingStream { continuation in
                if !chunks.isEmpty { continuation.yield(chunks.map(\.text)) }
                continuation.finish()
            }
        }
        let (stream, continuation) = AsyncThrowingStream<[String], any Error>.makeStream()
        let task = Task {
            let start = ContinuousClock.now
            for chunk in chunks {
                // Sleep until the chunk's arrival time, measured from the first.
                let due = start + .seconds(max(chunk.t - (chunks.first?.t ?? 0), 0))
                do {
                    try await Task.sleep(until: due, clock: .continuous)
                } catch {
                    continuation.finish(throwing: error)
                    return
                }
                continuation.yield([chunk.text])
            }
            continuation.finish()
        }
        continuation.onTermination = { _ in task.cancel() }
        return stream
    }
}

/// Where recordings are saved, and the ones there.
public enum Recordings {
    public static let folderKey = "recordingsFolder"

    /// The folder in settings, or the app's own folder in Application Support.
    public static var directory: URL {
        let folder = UserDefaults.standard.string(forKey: folderKey) ?? ""
        if !folder.isEmpty { return URL(fileURLWithPath: folder, isDirectory: true) }
        return defaultDirectory
    }

    public static var defaultDirectory: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("dev.openui-rs.viewer/Recordings", isDirectory: true)
    }

    public static let fileSuffix = ".recording.json"

    /// Saves a recording as `<time>-<first words>.recording.json`; returns its file.
    @discardableResult
    public static func save(_ recording: Recording, in directory: URL = directory) throws -> URL {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let stamp = ISO8601DateFormatter.string(from: recording.createdAt, timeZone: .gmt,
                                                 formatOptions: [.withFullDate, .withTime])
        let words = recording.request.lowercased()
            .components(separatedBy: CharacterSet.alphanumerics.inverted)
            .filter { !$0.isEmpty }.prefix(6).joined(separator: "-")
        let url = directory.appendingPathComponent("\(stamp)-\(words.isEmpty ? "recording" : words)\(fileSuffix)")
        try recording.encoded().write(to: url, options: .atomic)
        return url
    }

    /// The recordings in `directory`, newest first; files that don't decode are skipped.
    public static func list(in directory: URL = directory) -> [RecordingFile] {
        let urls = (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
        return urls
            .filter { $0.lastPathComponent.hasSuffix(fileSuffix) }
            .compactMap { url in
                (try? Recording.decode(Data(contentsOf: url))).map { RecordingFile(url: url, recording: $0) }
            }
            .sorted { $0.recording.createdAt > $1.recording.createdAt }
    }

    @MainActor public static func reveal(_ url: URL) {
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }
}

public struct RecordingFile: Identifiable, Sendable {
    public var url: URL
    public var recording: Recording
    /// The sidebar's selection tag.
    public var id: String { "recording:" + url.lastPathComponent }
    public var title: String { recording.request.split(whereSeparator: \.isNewline).first.map(String.init) ?? url.lastPathComponent }
}
