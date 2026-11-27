import CoreFoundation
import Darwin
import Foundation
import Synchronization

struct LoopbackHTTPRequest {
    let method: String
    let target: String
    let headers: [String: String]
    let body: Data
}

struct LoopbackHTTPResponse {
    var status: Int
    var headers: [String: String] = [:]
    var body = Data()
    var sent: @Sendable () -> Void = {}

    var wireBytes: Data {
        var header = "HTTP/1.1 \(status) Response\r\nConnection: close\r\nContent-Length: \(body.count)\r\n"
        for (name, value) in headers { header += "\(name): \(value)\r\n" }
        return Data((header + "\r\n").utf8) + body
    }
}

/// The connection budget, header and body bounds apply before parsing.
final class LoopbackHTTPListener: Sendable {
    typealias Shutdown = @Sendable (_ close: @Sendable () -> Void, _ acknowledge: @Sendable () -> Void) -> Void
    private typealias Handler = @Sendable (LoopbackHTTPRequest) -> LoopbackHTTPResponse

    private struct State {
        var remaining: Int
        var connections: [UUID: Int32] = [:]
        var handler: Handler?
    }

    private struct Lifecycle {
        var socket: CFSocket?
        var stopping = false
        var finished = false
    }

    private let port: UInt16
    private let lifecycle = Mutex(Lifecycle())
    private let stopped = AsyncThrowingStream<Void, any Error>.makeStream()
    private let shutdown: Shutdown
    private let state: Mutex<State>
    // Releasing a local socket needs no peer response. Allow a second for
    // dispatch, then invalidate the native socket directly from another queue.
    static let shutdownDeadline: DispatchTimeInterval = .seconds(1)
    private static let connectionDeadline: DispatchTimeInterval = .seconds(5)
    private static let maxHeaderBytes = 16 << 10
    private let maxBodyBytes: Int

    init(port: UInt16 = 0, maxConnections: Int = 16, maxBodyBytes: Int = 0,
         shutdown: @escaping Shutdown = { close, acknowledge in
             close()
             acknowledge()
         }) {
        self.port = port
        self.shutdown = shutdown
        state = Mutex(State(remaining: maxConnections))
        self.maxBodyBytes = maxBodyBytes
    }

    deinit { closeSocket() }

    func respond(_ handler: @escaping @Sendable (LoopbackHTTPRequest) -> LoopbackHTTPResponse) {
        state.withLock { $0.handler = handler }
    }

    func start() throws -> URL {
        try Task.checkCancellation()
        return try lifecycle.withLock { lifecycle in
            guard !lifecycle.stopping, lifecycle.socket == nil else { throw MCPFailure.listener }
            let (fd, origin) = try Self.bind(port)
            // The socket owns its descriptor. Keep only a weak target in its
            // callback context so abandoning the listener also releases it.
            let target = AcceptTarget(self)
            var context = CFSocketContext(version: 0, info: Unmanaged.passUnretained(target).toOpaque(),
                                          retain: { pointer in
                guard let pointer else { return nil }
                _ = Unmanaged<AcceptTarget>.fromOpaque(pointer).retain()
                return pointer
            }, release: { pointer in
                if let pointer { Unmanaged<AcceptTarget>.fromOpaque(pointer).release() }
            }, copyDescription: nil)
            guard let socket = CFSocketCreateWithNative(nil, fd, CFSocketCallBackType.acceptCallBack.rawValue,
                                                       { _, _, _, data, info in
                guard let data else { return }
                let accepted = data.load(as: CFSocketNativeHandle.self)
                if let info, let listener = Unmanaged<AcceptTarget>.fromOpaque(info).takeUnretainedValue().listener {
                    listener.accept(accepted)
                } else {
                    Darwin.close(accepted)
                }
            }, &context) else {
                Darwin.close(fd)
                throw MCPFailure.listener
            }
            CFSocketSetSocketFlags(socket, CFSocketGetSocketFlags(socket) | CFOptionFlags(kCFSocketCloseOnInvalidate))
            guard let source = CFSocketCreateRunLoopSource(nil, socket, 0) else {
                CFSocketInvalidate(socket)
                throw MCPFailure.listener
            }
            lifecycle.socket = socket
            CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
            return origin
        }
    }

    private final class AcceptTarget {
        weak var listener: LoopbackHTTPListener?
        init(_ listener: LoopbackHTTPListener) { self.listener = listener }
    }

    private static func bind(_ port: UInt16) throws -> (Int32, URL) {
        let fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
        guard fd >= 0 else { throw MCPFailure.listener }
        var transferred = false
        defer { if !transferred { Darwin.close(fd) } }
        var enabled: Int32 = 1
        guard fcntl(fd, F_SETFD, FD_CLOEXEC) != -1,
              setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &enabled, socklen_t(MemoryLayout<Int32>.size)) == 0
        else { throw MCPFailure.listener }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian
        address.sin_addr.s_addr = INADDR_LOOPBACK.bigEndian
        var size = socklen_t(address.sin_len)
        let result = withUnsafeMutablePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { address in
                Darwin.bind(fd, address, size) == 0 &&
                listen(fd, SOMAXCONN) == 0 &&
                getsockname(fd, address, &size) == 0
            }
        }
        guard result, let origin = URL(string: "http://127.0.0.1:\(UInt16(bigEndian: address.sin_port))")
        else { throw MCPFailure.listener }
        transferred = true
        return (fd, origin)
    }

    func stop() {
        let deadline = DispatchTime.now() + Self.shutdownDeadline
        let first = lifecycle.withLock { lifecycle in
            guard !lifecycle.stopping else { return false }
            lifecycle.stopping = true
            return true
        }
        guard first else { return }
        DispatchQueue.global().asyncAfter(deadline: deadline) { [weak self] in
            self?.finishShutdown(timedOut: true)
        }
        DispatchQueue.global(qos: .userInitiated).async {
            self.shutdown({ self.closeSocket() }, { self.finishShutdown(timedOut: false) })
        }
    }

    func stopAndWait() async throws {
        stop()
        // Caller cancellation starts shutdown, but must not cancel the wait
        // for the port to be released.
        try await Task {
            var iterator = stopped.stream.makeAsyncIterator()
            _ = try await iterator.next()
        }.value
    }

    private func finishShutdown(timedOut: Bool) {
        closeConnections()
        lifecycle.withLock { lifecycle in
            guard !lifecycle.finished else { return }
            if let socket = lifecycle.socket { CFSocketInvalidate(socket) }
            lifecycle.socket = nil
            lifecycle.finished = true
            stopped.continuation.finish(throwing: timedOut ? MCPFailure.listenerShutdownTimeout : nil)
        }
    }

    private func closeSocket() {
        closeConnections()
        lifecycle.withLock { lifecycle in
            if let socket = lifecycle.socket { CFSocketInvalidate(socket) }
            lifecycle.socket = nil
        }
    }

    private func closeConnections() {
        state.withLock { state in
            state.remaining = 0
            for fd in state.connections.values { Darwin.shutdown(fd, SHUT_RDWR) }
        }
    }

    private func accept(_ fd: Int32) {
        var enabled: Int32 = 1
        guard fcntl(fd, F_SETFD, FD_CLOEXEC) != -1,
              fcntl(fd, F_SETFL, 0) != -1,
              setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &enabled, socklen_t(MemoryLayout<Int32>.size)) == 0
        else { Darwin.close(fd); return }

        let id = UUID()
        let accepted = state.withLock { state in
            guard state.remaining > 0, state.connections.count < 8 else { return false }
            state.remaining -= 1
            state.connections[id] = fd
            return true
        }
        guard accepted else { Darwin.close(fd); return }
        DispatchQueue.global().asyncAfter(deadline: .now() + Self.connectionDeadline) { [weak self] in
            self?.state.withLock { state in
                if let fd = state.connections[id] { Darwin.shutdown(fd, SHUT_RDWR) }
            }
        }
        // Each worker handles one bounded request. Shutdown wakes both recv
        // and send, and only the worker closes the descriptor.
        DispatchQueue.global(qos: .userInitiated).async {
            defer {
                self.state.withLock { state in
                    state.connections.removeValue(forKey: id)
                    Darwin.close(fd)
                }
            }
            guard let request = self.read(fd), let respond = self.state.withLock({ $0.handler }) else { return }
            let response = respond(request)
            if Self.send(response.wireBytes, through: fd) { response.sent() }
        }
    }

    private func read(_ fd: Int32) -> LoopbackHTTPRequest? {
        let separator = Data("\r\n\r\n".utf8)
        var buffer = Data()
        // Every receive consumes at least one byte or ends the request.
        for _ in 0..<(Self.maxHeaderBytes + maxBodyBytes) {
            guard let bytes = Self.receive(from: fd) else { return nil }
            buffer.append(bytes)
            if let end = buffer.range(of: separator) {
                guard end.lowerBound <= Self.maxHeaderBytes,
                      let head = String(data: buffer.prefix(upTo: end.lowerBound), encoding: .utf8),
                      let parsed = Self.parse(head), parsed.length <= maxBodyBytes else { return nil }
                let prefix = buffer.suffix(from: end.upperBound)
                guard prefix.count <= parsed.length else { return nil }
                var body = Data(prefix)
                for _ in 0..<(parsed.length - body.count) {
                    if body.count == parsed.length { break }
                    guard let chunk = Self.receive(from: fd),
                          chunk.count <= parsed.length - body.count else { return nil }
                    body.append(chunk)
                }
                return LoopbackHTTPRequest(method: parsed.method, target: parsed.target, headers: parsed.headers, body: body)
            }
            if buffer.count >= Self.maxHeaderBytes { return nil }
        }
        return nil
    }

    private static func parse(_ head: String) -> (method: String, target: String, headers: [String: String], length: Int)? {
        let lines = head.components(separatedBy: "\r\n")
        let request = lines.first?.split(separator: " ") ?? []
        guard request.count == 3, let method = request.first,
              let target = request.dropFirst().first, target.hasPrefix("/") else { return nil }
        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            let pieces = line.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
            guard pieces.count == 2, let key = pieces.first?.lowercased(), let value = pieces.last,
                  headers[key] == nil else { return nil }
            headers[key] = value.trimmingCharacters(in: .whitespaces)
        }
        guard headers["transfer-encoding"] == nil,
              let length = Int(headers["content-length"] ?? "0"), length >= 0 else { return nil }
        return (String(method), String(target), headers, length)
    }

    private static func receive(from fd: Int32) -> Data? {
        var bytes = Data(count: 4096)
        let count = bytes.withUnsafeMutableBytes { recv(fd, $0.baseAddress, $0.count, 0) }
        return count > 0 ? bytes.prefix(count) : nil
    }

    private static func send(_ data: Data, through fd: Int32) -> Bool {
        data.withUnsafeBytes { bytes in
            guard let start = bytes.baseAddress else { return false }
            var sent = 0
            // Each successful write consumes at least one byte.
            for _ in 0..<bytes.count {
                let count = Darwin.send(fd, start.advanced(by: sent), bytes.count - sent, 0)
                guard count > 0 else { return false }
                sent += count
                if sent == bytes.count { return true }
            }
            return false
        }
    }
}
