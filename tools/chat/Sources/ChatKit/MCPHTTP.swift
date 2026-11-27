import Foundation
import OpenUINative
import Synchronization

/// Gives the SDK a bounded response before it starts decoding JSON or SSE.
/// The inner session makes the actual requests and keeps credentials in memory.
final class MCPHTTP: URLProtocol, @unchecked Sendable {
    private let operation = Mutex<Task<Void, Never>?>(nil)

    static func configuration() -> URLSessionConfiguration {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 20
        config.timeoutIntervalForResource = 30
        config.httpCookieStorage = nil
        config.urlCredentialStorage = nil
        config.urlCache = nil
        config.connectionProxyDictionary = [:]
        return config
    }

    static func sdkConfiguration() -> URLSessionConfiguration {
        let config = configuration()
        config.protocolClasses = [MCPHTTP.self]
        return config
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        operation.withLock { operation in
            operation = Task { await self.forwardBoundedResponse() }
        }
    }

    override func stopLoading() {
        operation.withLock { operation in
            operation?.cancel()
            operation = nil
        }
    }

    private func forwardBoundedResponse() async {
        do {
            let (data, response) = try await Self.read(request)
            try Task.checkCancellation()
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            if !data.isEmpty { client?.urlProtocol(self, didLoad: data) }
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            if !Task.isCancelled { client?.urlProtocol(self, didFailWithError: error) }
        }
    }

    static func read(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let session = URLSession(configuration: configuration(), delegate: NoRedirect(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        let (bytes, response) = try await session.bytes(for: request)
        guard let http = response as? HTTPURLResponse else { throw MCPFailure.connection }
        let successful = (200..<300).contains(http.statusCode)
        let limit = successful ? defaultHostByteLimit : ThesysGateway.maxErrorBodyBytes
        var body = Data()
        // Success keeps a complete JSON or SSE body. An error body is capped
        // before it reaches the SDK, and its text never goes into the UI.
        var reader = bytes.makeAsyncIterator()
        for _ in 0..<limit {
            guard let byte = try await reader.next() else { return (body, http) }
            body.append(byte)
        }
        if successful, try await reader.next() != nil { throw MCPFailure.responseLimit }
        return (body, http)
    }
}

private final class NoRedirect: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest) async -> URLRequest? { nil }
}
