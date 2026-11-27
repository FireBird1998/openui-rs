import CryptoKit
import Darwin
import Foundation
import OpenUINative
import Synchronization
@testable import ChatKit

/// A real HTTP endpoint in the test process. It implements two pages of
/// Streamable HTTP tools, RFC discovery and a synthetic OAuth provider.
final class MCPFixture: Sendable {
    enum ToolListFailure: CaseIterable {
        case malformedJSON, wrongContentType
    }

    private struct State {
        var redirect: URL?
        var challenge: String?
        var issuerOverride: URL?
        var generation = 0
        var expired = false
        var refuse = false
        var refuseCode = false
        var denyAuthorization = false
    }

    private let state = Mutex(State())
    private let listener: LoopbackHTTPListener
    private let origin: URL
    let endpoint: URL
    let registrations = Counter()
    let authorizations = Counter()
    let codeExchanges = Counter()
    let refreshes = Counter()
    let getsAtMCP = Counter()
    let invalidToolReplies = Counter()
    private let isProtected: Bool
    private let toolNames: [String]
    private let oversized: Bool
    private let toolListFailure: ToolListFailure?
    private let authorizationEndpoint: URL?
    private let beforeRegistrationReply: @Sendable () -> Void

    static func start(protected: Bool = false, toolNames: [String] = ["lookup", "search"],
                      oversized: Bool = false, toolListFailure: ToolListFailure? = nil,
                      authorizationEndpoint: URL? = nil, beforeRegistrationReply: @escaping @Sendable () -> Void = {}) async throws -> MCPFixture {
        let listener = LoopbackHTTPListener(maxConnections: 80, maxBodyBytes: 32 << 10)
        do {
            let origin = try listener.start()
            let server = MCPFixture(listener: listener, origin: origin, protected: protected,
                                    toolNames: toolNames, oversized: oversized, toolListFailure: toolListFailure,
                                    authorizationEndpoint: authorizationEndpoint, beforeRegistrationReply: beforeRegistrationReply)
            listener.respond { [weak server] request in
                server?.response(to: request) ?? .init(status: 503)
            }
            return server
        } catch {
            listener.stop()
            throw error
        }
    }

    private init(listener: LoopbackHTTPListener, origin: URL, protected: Bool,
                 toolNames: [String], oversized: Bool, toolListFailure: ToolListFailure?,
                 authorizationEndpoint: URL?, beforeRegistrationReply: @escaping @Sendable () -> Void) {
        self.listener = listener
        self.origin = origin
        endpoint = origin.appending(path: "mcp")
        isProtected = protected
        self.toolNames = toolNames
        self.oversized = oversized
        self.toolListFailure = toolListFailure
        self.authorizationEndpoint = authorizationEndpoint
        self.beforeRegistrationReply = beforeRegistrationReply
    }

    func stop() { listener.stop() }
    func expireAccessToken() { state.withLock { $0.expired = true } }
    func refuseRefresh(_ refuse: Bool) { state.withLock { $0.refuse = refuse } }
    func refuseCode(_ refuse: Bool) { state.withLock { $0.refuseCode = refuse } }
    func denyAuthorization(_ deny: Bool) { state.withLock { $0.denyAuthorization = deny } }
    func advertiseIssuer(of other: MCPFixture) { state.withLock { $0.issuerOverride = other.origin } }

    func openBrowser(_ url: URL) async throws {
        guard url.scheme == origin.scheme, url.host == origin.host, url.port == origin.port else {
            throw MCPFailure.browser
        }
        let session = URLSession(configuration: MCPHTTP.configuration())
        defer { session.invalidateAndCancel() }
        // A request carrying the wrong state is rejected and leaves the
        // listener ready for the provider's real redirect.
        if let redirect = state.withLock({ $0.redirect }),
           let wrong = URL(string: redirect.absoluteString + "?code=synthetic-wrong&state=wrong") {
            let (_, rejected) = try await session.data(from: wrong)
            guard (rejected as? HTTPURLResponse)?.statusCode == 400 else { throw MCPFailure.browser }
        }
        let (_, response) = try await session.data(from: url)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw MCPFailure.browser }
    }

    func callbackIsClosed() async throws -> Bool {
        guard let redirect = state.withLock({ $0.redirect }) else { return false }
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 1
        let session = URLSession(configuration: config)
        defer { session.invalidateAndCancel() }
        for _ in 0..<20 {
            do { _ = try await session.data(from: redirect) }
            catch let error as URLError where error.code == .cannotConnectToHost { return true }
            try await Task.sleep(for: .milliseconds(20))
        }
        return false
    }

    func callbackAcceptsTCP() throws -> Bool {
        var address = try callbackAddress()
        let fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
        guard fd >= 0 else { throw MCPFailure.listener }
        defer { Darwin.close(fd) }
        let size = socklen_t(address.sin_len)
        let result = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, size) }
        }
        if result == 0 { return true }
        if errno == ECONNREFUSED { return false }
        throw MCPFailure.listener
    }

    struct ListeningSocket: Hashable, Sendable {
        let descriptor: Int32
        let port: UInt16
    }

    static func listeningTCPSockets() throws -> Set<ListeningSocket> {
        let pid = getpid()
        let stride = MemoryLayout<proc_fdinfo>.stride
        let estimate = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, nil, 0)
        guard estimate > 0 else { throw MCPFailure.listener }
        var descriptors = [proc_fdinfo](repeating: proc_fdinfo(), count: Int(estimate) / stride + 32)
        let bytes = descriptors.withUnsafeMutableBytes {
            proc_pidinfo(pid, PROC_PIDLISTFDS, 0, $0.baseAddress, Int32($0.count))
        }
        guard bytes > 0, Int(bytes) < descriptors.count * stride else { throw MCPFailure.listener }

        var listening: Set<ListeningSocket> = []
        for entry in descriptors.prefix(Int(bytes) / stride) where entry.proc_fdtype == PROX_FDTYPE_SOCKET {
            var info = socket_fdinfo()
            let size = Int32(MemoryLayout<socket_fdinfo>.size)
            let result = proc_pidfdinfo(pid, entry.proc_fd, PROC_PIDFDSOCKETINFO, &info, size)
            // A client connection may have closed since the descriptor snapshot.
            if result == 0 && (errno == EBADF || errno == ENOTSOCK) { continue }
            guard result == size else { throw MCPFailure.listener }
            guard info.psi.soi_kind == SOCKINFO_TCP else { continue }
            let tcp = info.psi.soi_proto.pri_tcp
            if tcp.tcpsi_state == TSI_S_LISTEN {
                let port = UInt16(bigEndian: UInt16(truncatingIfNeeded: tcp.tcpsi_ini.insi_lport))
                listening.insert(ListeningSocket(descriptor: entry.proc_fd, port: port))
            }
        }
        return listening
    }

    func callbackPortCanRebind() throws -> Bool {
        var address = try callbackAddress()
        let fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
        guard fd >= 0 else { throw MCPFailure.listener }
        defer { Darwin.close(fd) }
        var reuse: Int32 = 1
        guard setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &reuse, socklen_t(MemoryLayout.size(ofValue: reuse))) == 0
        else { throw MCPFailure.listener }
        let size = socklen_t(address.sin_len)
        let result = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(fd, $0, size) }
        }
        if result == 0 { return listen(fd, SOMAXCONN) == 0 }
        if errno == EADDRINUSE { return false }
        throw MCPFailure.listener
    }

    func issuedRegistration() throws -> MCPRegistration {
        guard let redirect = state.withLock({ $0.redirect }) else { throw MCPFailure.registration }
        return MCPRegistration(issuer: origin, tokenEndpoint: origin.appending(path: "token"), redirect: redirect,
                               clientID: "synthetic-client", method: "none", clientSecret: nil, secretExpiry: nil)
    }

    private func callbackAddress() throws -> sockaddr_in {
        guard let port = state.withLock({ $0.redirect?.port }).flatMap(UInt16.init(exactly:)) else {
            throw MCPFailure.listener
        }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian
        address.sin_addr.s_addr = INADDR_LOOPBACK.bigEndian
        return address
    }

    func callbackResponds() async throws -> Bool {
        guard let redirect = state.withLock({ $0.redirect }) else { return false }
        let session = URLSession(configuration: MCPHTTP.configuration())
        defer { session.invalidateAndCancel() }
        let (_, response) = try await session.data(from: redirect)
        return (response as? HTTPURLResponse)?.statusCode == 400
    }

    private func response(to request: LoopbackHTTPRequest) -> LoopbackHTTPResponse {
        guard let target = URLComponents(string: request.target) else { return .init(status: 400) }
        switch (request.method, target.path) {
        case ("GET", "/mcp"):
            getsAtMCP.add(1)
            return .init(status: 405)
        case ("POST", "/mcp"):
            let authenticated = state.withLock { state in
                !state.expired && state.generation > 0 &&
                request.headers["authorization"] == "Bearer synthetic-access-\(state.generation)"
            }
            if isProtected && !authenticated {
                return .init(status: 401, headers: [
                    "WWW-Authenticate": "Bearer resource_metadata=\"\(origin.appending(path: "resource-metadata"))\""
                ])
            }
            return rpc(request.body)
        case ("GET", "/resource-metadata"):
            let issuer = state.withLock { $0.issuerOverride } ?? origin
            return json(["resource": endpoint.absoluteString, "authorization_servers": [issuer.absoluteString]])
        case ("GET", "/.well-known/oauth-authorization-server"):
            return json([
                "issuer": origin.absoluteString,
                "authorization_endpoint": (authorizationEndpoint ?? origin.appending(path: "authorize")).absoluteString,
                "registration_endpoint": origin.appending(path: "register").absoluteString,
                "token_endpoint": origin.appending(path: "token").absoluteString, "code_challenge_methods_supported": ["S256"],
                "token_endpoint_auth_methods_supported": ["none"],
            ])
        case ("POST", "/register"):
            return register(request.body)
        case ("GET", "/authorize"):
            return authorize(target.queryItems ?? [])
        case ("POST", "/token"):
            return token(request.body)
        default:
            return .init(status: 404)
        }
    }

    private func rpc(_ body: Data) -> LoopbackHTTPResponse {
        guard let request = (try? JSONSerialization.jsonObject(with: body)) as? [String: Any],
              let method = request["method"] as? String else { return .init(status: 400) }
        if method == "notifications/initialized" { return .init(status: 202) }
        guard let id = request["id"] else { return .init(status: 400) }
        var result: [String: Any]
        switch method {
        case "initialize":
            result = ["protocolVersion": "2025-11-25", "serverInfo": ["name": "Local fixture", "version": "1"],
                      "capabilities": ["tools": [:] as [String: String]]]
        case "tools/list":
            let cursor = (request["params"] as? [String: Any])?["cursor"] as? String
            if oversized {
                return .init(status: 200, headers: ["Content-Type": "application/json"],
                             body: Data(repeating: 32, count: defaultHostByteLimit + 1))
            }
            if toolListFailure == .malformedJSON {
                return .init(status: 200, headers: ["Content-Type": "application/json"],
                             body: Data("{".utf8), sent: { self.invalidToolReplies.add(1) })
            }
            if toolListFailure == .wrongContentType,
               let body = try? JSONSerialization.data(withJSONObject: [
                   "jsonrpc": "2.0", "id": id, "result": ["tools": [] as [String]],
               ]) {
                return .init(status: 200, headers: ["Content-Type": "text/html"],
                             body: body, sent: { self.invalidToolReplies.add(1) })
            }
            let names = cursor == nil ? toolNames.prefix(1) : toolNames.dropFirst()
            result = ["tools": names.map { ["name": $0, "inputSchema": ["type": "object"]] }]
            if cursor == nil { result["nextCursor"] = "page-2" }
        default:
            return .init(status: 400)
        }
        if method == "tools/list", (request["params"] as? [String: Any])?["cursor"] != nil,
           let body = try? JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": id, "result": result]) {
            // The standalone GET returns 405, while a POST can carry SSE.
            return .init(status: 200, headers: ["Content-Type": "text/event-stream"],
                         body: Data(("data: " + String(decoding: body, as: UTF8.self) + "\n\n").utf8))
        }
        return json(["jsonrpc": "2.0", "id": id, "result": result])
    }

    private func register(_ body: Data) -> LoopbackHTTPResponse {
        registrations.add(1)
        guard let request = (try? JSONSerialization.jsonObject(with: body)) as? [String: Any],
              let uri = (request["redirect_uris"] as? [String])?.first, let redirect = URL(string: uri),
              redirect.scheme == "http", redirect.host == "127.0.0.1", redirect.port != nil else {
            return .init(status: 400)
        }
        state.withLock { $0.redirect = redirect }
        beforeRegistrationReply()
        return json(["client_id": "synthetic-client", "redirect_uris": [uri],
                     "token_endpoint_auth_method": "none"], status: 201)
    }

    private func authorize(_ items: [URLQueryItem]) -> LoopbackHTTPResponse {
        authorizations.add(1)
        guard let redirect = state.withLock({ $0.redirect }),
              value("redirect_uri", in: items) == redirect.absoluteString,
              value("client_id", in: items) == "synthetic-client",
              value("code_challenge_method", in: items) == "S256",
              value("resource", in: items) == endpoint.absoluteString,
              let challenge = value("code_challenge", in: items), let nonce = value("state", in: items),
              var callback = URLComponents(url: redirect, resolvingAgainstBaseURL: false) else {
            return .init(status: 400)
        }
        let denied = state.withLock {
            $0.challenge = challenge
            return $0.denyAuthorization
        }
        let answer = denied ? URLQueryItem(name: "error", value: "access_denied")
                            : URLQueryItem(name: "code", value: "synthetic-code")
        callback.queryItems = [answer, .init(name: "state", value: nonce)]
        guard let location = callback.url?.absoluteString else { return .init(status: 400) }
        return .init(status: 302, headers: ["Location": location])
    }

    private func token(_ body: Data) -> LoopbackHTTPResponse {
        let text = String(decoding: body, as: UTF8.self).replacingOccurrences(of: "+", with: "%20")
        let items = URLComponents(string: "/?" + text)?.queryItems ?? []
        guard value("client_id", in: items) == "synthetic-client",
              value("resource", in: items) == endpoint.absoluteString else { return .init(status: 400) }
        return state.withLock { state in
            switch value("grant_type", in: items) {
            case "authorization_code":
                codeExchanges.add(1)
                guard !state.refuseCode, value("code", in: items) == "synthetic-code",
                      value("redirect_uri", in: items) == state.redirect?.absoluteString,
                      let verifier = value("code_verifier", in: items),
                      Data(SHA256.hash(data: Data(verifier.utf8))).base64EncodedString()
                        .replacingOccurrences(of: "+", with: "-")
                        .replacingOccurrences(of: "/", with: "_")
                        .replacingOccurrences(of: "=", with: "") == state.challenge else {
                    return .init(status: 400)
                }
            case "refresh_token":
                refreshes.add(1)
                guard !state.refuse, value("refresh_token", in: items) == "synthetic-refresh-\(state.generation)" else {
                    return json(["error": "invalid_grant"], status: 400)
                }
            default:
                return .init(status: 400)
            }
            state.generation += 1
            state.expired = false
            return json(["access_token": "synthetic-access-\(state.generation)", "token_type": "Bearer",
                         "refresh_token": "synthetic-refresh-\(state.generation)", "expires_in": 3600])
        }
    }

    private func value(_ key: String, in items: [URLQueryItem]) -> String? {
        items.first(where: { $0.name == key })?.value
    }

    private func json(_ object: [String: Any], status: Int = 200) -> LoopbackHTTPResponse {
        guard let body = try? JSONSerialization.data(withJSONObject: object) else { return .init(status: 500) }
        return .init(status: status, headers: ["Content-Type": "application/json"], body: body)
    }
}
