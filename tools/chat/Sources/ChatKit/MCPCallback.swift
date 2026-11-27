import Foundation
import Synchronization

/// One authorization answer, tied to the state and redirect registered for
/// this browser flow. Invalid requests leave the callback available.
struct MCPCallback: Sendable {
    typealias Start = @Sendable (String, URL?) async throws -> MCPCallback
    let redirect: URL
    private let listener: LoopbackHTTPListener
    private let codes: AsyncStream<Result<String, MCPFailure>>
    private let timeout: Duration

    static func start(state: String, preferred: URL?, timeout: Duration = .seconds(120),
                      makeListener: (UInt16) -> LoopbackHTTPListener = { LoopbackHTTPListener(port: $0) }) async throws -> MCPCallback {
        // A registered client is tied to its redirect. If the port is
        // occupied, return the bind failure so a retry can reuse the client.
        let port: UInt16
        if let preferred {
            guard let saved = preferred.port.flatMap(UInt16.init(exactly:)), saved != 0 else { throw MCPFailure.listener }
            port = saved
        } else {
            port = 0
        }
        let listener = makeListener(port)
        do {
            let origin = try listener.start()
            let redirect = origin.appending(path: "callback")
            guard preferred == nil || redirect == preferred else { throw MCPFailure.listener }
            let (codes, pending) = AsyncStream<Result<String, MCPFailure>>.makeStream(bufferingPolicy: .bufferingOldest(1))
            let completed = Mutex(false)
            listener.respond { request in
                guard request.method == "GET", request.headers["host"] == "127.0.0.1:\(origin.port ?? 0)",
                      let target = URLComponents(string: request.target), target.host == nil,
                      target.fragment == nil, target.percentEncodedPath == "/callback",
                      let query = target.queryItems, Self.value("state", in: query) == state else {
                    return LoopbackHTTPResponse(status: 400)
                }
                let reply: Result<String, MCPFailure>
                let message: String
                if Self.value("error", in: query) != nil {
                    reply = .failure(.denied)
                    message = "Sign-in was declined. You can close this window."
                } else if let code = Self.value("code", in: query), !code.isEmpty {
                    reply = .success(code)
                    message = "Sign-in returned to OpenUI Chat. You can close this window."
                } else {
                    return LoopbackHTTPResponse(status: 400)
                }
                let first = completed.withLock { completed in
                    defer { completed = true }
                    return !completed
                }
                guard first else { return LoopbackHTTPResponse(status: 400) }
                return LoopbackHTTPResponse(status: 200, headers: [
                    "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store",
                ], body: Data(message.utf8), sent: {
                    pending.yield(reply)
                    pending.finish()
                })
            }
            return MCPCallback(redirect: redirect, listener: listener, codes: codes, timeout: timeout)
        } catch {
            try await listener.stopAndWait()
            throw error
        }
    }

    func withListener<Value>(_ operation: () async throws -> Value) async throws -> Value {
        let value: Value
        do {
            value = try await withTaskCancellationHandler(operation: operation) { stop() }
        } catch {
            try await listener.stopAndWait()
            throw error
        }
        try await listener.stopAndWait()
        return value
    }

    func waitForCode(opening url: URL, with browser: @escaping MCPBrowser) async throws -> String {
        try await withThrowingTaskGroup(of: String.self) { group in
            group.addTask {
                try await browser(url)
                var pending = codes.makeAsyncIterator()
                guard let result = await pending.next() else { throw CancellationError() }
                return try result.get()
            }
            group.addTask {
                try await Task.sleep(for: timeout)
                throw MCPFailure.signInTimeout
            }
            defer { group.cancelAll() }
            guard let code = try await group.next() else { throw CancellationError() }
            return code
        }
    }

    func stop() { listener.stop() }

    private static func value(_ name: String, in query: [URLQueryItem]) -> String? {
        let matches = query.filter { $0.name == name }
        return matches.count == 1 ? matches.first?.value : nil
    }
}
