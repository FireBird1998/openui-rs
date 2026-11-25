import Foundation
import Synchronization

// Swift Testing 2084 can report the same start/end pair for an entire
// repeated function. Emit a structured completion from the real test body
// after its checks, then let Swift Testing certify that all checks passed.
struct TextRepetition {
    private struct Completion: Encodable {
        let test: String
        let iteration: Int
        let elapsed: TimeInterval
    }

    private static let path = ProcessInfo.processInfo.environment["OPENUI_SWIFT_TEST_REPRO_EVENTS"]
    private static let completed = Mutex<[String: Int]>([:])
    private let start = ProcessInfo.processInfo.systemUptime

    func finish(_ test: String = #function) throws {
        guard let path = Self.path else { return }
        let elapsed = ProcessInfo.processInfo.systemUptime - start
        try Self.completed.withLock { completed in
            let next = completed[test, default: 0] + 1
            var record = try JSONEncoder().encode(Completion(test: test, iteration: next, elapsed: elapsed))
            record.append(0x0A)
            let stream = try FileHandle(forWritingTo: URL(fileURLWithPath: path))
            defer { try? stream.close() }
            try stream.seekToEnd()
            try stream.write(contentsOf: record)
            completed[test] = next
        }
    }
}
