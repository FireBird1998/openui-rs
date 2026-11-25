import Foundation
import OpenUINative
import Synchronization
@testable import ChatKit

/// A made-up key for tests. No test reads or writes a real key or the
/// Keychain.
let testKey = "test-key-not-real-0000"

/// The key store tests use in place of the Keychain.
final class InMemoryKeyStore: APIKeyStore {
    private let key: Mutex<String?>
    init(_ key: String? = nil) { self.key = Mutex(key) }
    func hasKey() -> Bool { key.withLock { $0 != nil } }
    func read() -> String? { key.withLock { $0 } }
    func save(_ key: String) { self.key.withLock { $0 = key } }
    func delete() { key.withLock { $0 = nil } }
}

/// One recorded HTTP exchange from `Recordings/<name>.json`: a status and
/// the body in the pieces it arrived in. With `hold`, the body waits after
/// its pieces until the request is cancelled; with `failAfter`, the
/// connection fails with that text.
struct Recording: Sendable {
    var status: Int
    var chunks: [String]
    var hold = false
    var failAfter: String?
    // Gates let a test release a held stream or postpone the HTTP response.
    var release: Flag?
    var beforeResponse: Flag?

    static func named(_ name: String) throws -> Recording {
        guard let url = Bundle.module.url(forResource: name, withExtension: "json", subdirectory: "Recordings") else {
            throw GatewayError("no recording named \(name)")
        }
        let json = try JSONValue.parse(String(contentsOf: url, encoding: .utf8))
        return Recording(
            status: Int(json["status"]?.number ?? 0),
            chunks: (json["chunks"]?.array ?? []).compactMap(\.string),
            hold: json["hold"]?.bool ?? false,
            failAfter: json["failAfter"]?.string)
    }
}

/// A single response that counts every pull, including a pull that parks.
struct ProbedTransport: HTTPTransport {
    let status: Int
    let bytes: [UInt8]
    let remaining: AsyncStream<UInt8>?
    let pulls = Counter()
    let released = Flag()

    init(status: Int, body: String, remaining: AsyncStream<UInt8>? = nil) {
        self.status = status
        bytes = Array(body.utf8)
        self.remaining = remaining
    }

    func send(_ request: URLRequest) async throws -> HTTPResponse {
        HTTPResponse(status: status, body: Body(transport: self))
    }

    private struct Body: AsyncSequence {
        let transport: ProbedTransport

        func makeAsyncIterator() -> Iterator {
            Iterator(bytes: transport.bytes.makeIterator(), remaining: transport.remaining?.makeAsyncIterator(),
                     pulls: transport.pulls, releaser: Releaser(transport.released))
        }

        struct Iterator: AsyncIteratorProtocol {
            var bytes: IndexingIterator<[UInt8]>
            var remaining: AsyncStream<UInt8>.Iterator?
            let pulls: Counter
            let releaser: Releaser

            mutating func next() async throws -> UInt8? {
                pulls.add(1)
                if let byte = bytes.next() { return byte }
                return await remaining?.next()
            }
        }
    }
}

/// Replays recordings in order, one per request, and keeps the requests it
/// was sent.
final class RecordedTransport: HTTPTransport {
    private let state: Mutex<(recordings: [Recording], sent: [URLRequest])>
    /// Set once a held body has delivered its pieces and is waiting.
    let holding = Flag()
    /// Set once the app ended a body before it finished.
    let cancelled = Flag()

    init(_ recordings: Recording...) {
        state = Mutex((recordings, []))
    }

    var sent: [URLRequest] { state.withLock { $0.sent } }

    func send(_ request: URLRequest) async throws -> HTTPResponse {
        let recording = try state.withLock { state throws -> Recording in
            state.sent.append(request)
            guard !state.recordings.isEmpty else { throw GatewayError("no recording left") }
            return state.recordings.removeFirst()
        }
        if let gate = recording.beforeResponse {
            while !gate.isSet { try await Task.sleep(for: .milliseconds(2)) }
        }
        let holding = self.holding
        let cancelled = self.cancelled
        let body = AsyncThrowingStream<UInt8, any Error> { continuation in
            let task = Task {
                for chunk in recording.chunks {
                    for byte in chunk.utf8 { continuation.yield(byte) }
                    await Task.yield()
                }
                if let failure = recording.failAfter {
                    continuation.finish(throwing: URLError(.networkConnectionLost, userInfo: [NSLocalizedDescriptionKey: failure]))
                    return
                }
                if recording.hold {
                    holding.set()
                    while !Task.isCancelled && recording.release?.isSet != true {
                        try? await Task.sleep(for: .milliseconds(5))
                    }
                }
                continuation.finish()
            }
            continuation.onTermination = { termination in
                if case .cancelled = termination { cancelled.set() }
                task.cancel()
            }
        }
        return HTTPResponse(status: recording.status, body: body)
    }
}

final class Flag: Sendable {
    private let value = Mutex(false)
    func set() { value.withLock { $0 = true } }
    var isSet: Bool { value.withLock { $0 } }
}

/// Waits, a bounded number of times, for `condition`.
@MainActor
func eventually(_ condition: @MainActor () -> Bool) async -> Bool {
    for _ in 0..<2000 {
        if condition() { return true }
        try? await Task.sleep(for: .milliseconds(2))
    }
    return condition()
}

extension URLRequest {
    var bodyText: String { httpBody.map { String(decoding: $0, as: UTF8.self) } ?? "" }
}

/// An answer that never ends: a small program, then a statement after
/// statement, each made only when the reader asks for its bytes. It counts
/// the bytes read and notices when the reader lets the body go, which is how
/// the request is cancelled.
final class EndlessTransport: HTTPTransport {
    static let head = "root = Stack([title])\ntitle = TextContent(\"Endless\")\n"
    let bytesRead = Counter()
    let released = Flag()
    /// How many characters each statement's string holds after the head.
    let filler: Int

    /// When supplied, each permit releases one delta, including the head.
    private let permits: AsyncStream<Void>?

    init(filler: Int = 200, permits: AsyncStream<Void>? = nil) {
        self.filler = filler
        self.permits = permits
    }

    func send(_ request: URLRequest) async throws -> HTTPResponse {
        let body = EndlessBody(bytesRead: bytesRead, released: released, filler: filler, permits: permits)
        return HTTPResponse(status: 200, body: body)
    }
}

struct EndlessBody: AsyncSequence {
    typealias Element = UInt8
    let bytesRead: Counter
    let released: Flag
    let filler: Int
    let permits: AsyncStream<Void>?

    struct AsyncIterator: AsyncIteratorProtocol {
        let bytesRead: Counter
        let releaser: Releaser
        let filler: Int
        var permits: AsyncStream<Void>.Iterator?
        var pending: [UInt8] = []
        var index = 0
        var statement = 0

        mutating func next() async throws -> UInt8? {
            if index == pending.count {
                if permits != nil, await permits?.next() == nil { return nil }
                let content = statement == 0 ? EndlessTransport.head
                    : "f\(statement) = \"" + String(repeating: "x", count: filler) + "\"\n"
                let event = JSONValue.object([("choices", .array([.object([("delta", .object([("content", .string(content))]))])]))])
                pending = Array(("data: " + event.compact + "\n\n").utf8)
                index = 0
                statement += 1
            }
            defer { index += 1 }
            bytesRead.add(1)
            return pending[index]
        }
    }

    func makeAsyncIterator() -> AsyncIterator {
        AsyncIterator(bytesRead: bytesRead, releaser: Releaser(released), filler: filler,
                      permits: permits?.makeAsyncIterator())
    }
}

/// Sets its flag when it is let go.
final class Releaser: Sendable {
    let flag: Flag
    init(_ flag: Flag) { self.flag = flag }
    deinit { flag.set() }
}

final class Counter: Sendable {
    private let value = Mutex(0)
    func add(_ n: Int) { value.withLock { $0 += n } }
    var count: Int { value.withLock { $0 } }
}
