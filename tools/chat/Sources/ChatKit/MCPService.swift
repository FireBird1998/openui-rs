import AppKit
import Foundation
import MCP

enum MCPAddress {
    private static let maxBytes = 2048

    static func parse(_ address: String) -> URL? {
        guard address.utf8.prefix(maxBytes + 1).count <= maxBytes,
              var parts = URLComponents(string: address.trimmingCharacters(in: .whitespacesAndNewlines)) else { return nil }
        parts.scheme = parts.scheme?.lowercased()
        parts.host = parts.host?.lowercased()
        if parts.port == 443, parts.scheme == "https" { parts.port = nil }
        if parts.percentEncodedPath.isEmpty { parts.path = "/" }
        guard let url = parts.url, valid(url) else { return nil }
        return url
    }

    static func valid(_ url: URL, allowsQuery: Bool = false) -> Bool {
        guard url.absoluteString.utf8.prefix(maxBytes + 1).count <= maxBytes,
              let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let host = parts.host, !host.isEmpty, parts.user == nil, parts.password == nil,
              parts.fragment == nil, (allowsQuery || parts.query == nil),
              parts.port.map({ (1...65535).contains($0) }) != false else { return false }
        return parts.scheme == "https" || (parts.scheme == "http" && host == "127.0.0.1")
    }
}

enum MCPFailure: LocalizedError, Equatable {
    case address, credentials, listener, listenerShutdownTimeout, browser, discovery, issuerChanged, registration
    case signIn, signInRequired, denied, signInTimeout, refresh, connection, responseLimit, toolLimit, busy
    case duplicateTool(String, URL, URL)
    case configured, serverLimit

    var errorDescription: String? {
        switch self {
        case .address: "Enter an HTTPS server address or an HTTP address on 127.0.0.1, without credentials, a query or a fragment."
        case .credentials: "The MCP sign-in could not be read or saved in the Keychain."
        case .listener: "The sign-in listener could not start on 127.0.0.1."
        case .listenerShutdownTimeout: "The sign-in listener took too long to stop. Its port was closed. Try again."
        case .browser: "The browser could not open the MCP sign-in."
        case .discovery: "The server did not provide usable OAuth metadata for PKCE sign-in."
        case .issuerChanged: "The server's sign-in provider changed. Remove and add the server again to use the new provider."
        case .registration: "The sign-in provider did not register OpenUI Chat."
        case .signIn: "The MCP sign-in failed."
        case .signInRequired: "Sign in to this server to list its tools."
        case .denied: "MCP sign-in was declined."
        case .signInTimeout: "The browser did not return to OpenUI Chat in time."
        case .refresh: "The server could not refresh the sign-in. The saved sign-in is still in the Keychain. Try again or sign in again."
        case .connection: "The MCP server could not list its tools. Check its address and try again."
        case .responseLimit: "The MCP server sent a response larger than the host input limit."
        case .toolLimit: "The MCP server sent an invalid or oversized tool list."
        case .busy: "A request for this MCP server is already running."
        case let .duplicateTool(name, existing, added):
            "Tool “\(name)” from \(added.absoluteString) conflicts with a tool from \(existing.absoluteString). The tool list was refused."
        case .configured: "This MCP server is already in Settings."
        case .serverLimit: "Settings can hold up to \(MCPSettings.maxServers) MCP servers."
        }
    }
}

actor MCPService {
    private let store: any MCPCredentialStore
    private let browser: MCPBrowser
    private let startCallback: MCPCallback.Start
    static let maxTools = 1024
    static let maxToolNameBytes = 128

    init(store: any MCPCredentialStore = KeychainMCPStore(),
         startCallback: @escaping MCPCallback.Start = { try await MCPCallback.start(state: $0, preferred: $1) },
         browser: @escaping MCPBrowser = { url in
             guard await MainActor.run(body: { NSWorkspace.shared.open(url) }) else { throw MCPFailure.browser }
         }) {
        self.store = store
        self.browser = browser
        self.startCallback = startCallback
    }

    func tools(at endpoint: URL, interactive: Bool = true, signInAgain: Bool = false) async throws -> [String] {
        guard MCPAddress.valid(endpoint) else { throw MCPFailure.address }
        try Task.checkCancellation()
        try store.attempts.begin(endpoint)
        defer { store.attempts.end(endpoint) }
        let authorization = try MCPAuthorization(endpoint: endpoint, store: store,
                                                 browser: interactive ? browser : nil, signInAgain: signInAgain,
                                                 startCallback: startCallback)
        // A standalone GET is optional in Streamable HTTP. POST replies
        // still accept SSE. This also avoids SDK background GET retries.
        let transport = HTTPClientTransport(endpoint: endpoint, configuration: MCPHTTP.sdkConfiguration(),
                                            streaming: false, authorizer: authorization)
        let client = Client(name: "OpenUI Chat", version: "1")
        do {
            return try await withTaskCancellationHandler {
                try await withThrowingTaskGroup(of: [String].self) { group in
                    group.addTask { try await Self.list(client, over: transport, endpoint: endpoint) }
                    group.addTask {
                        // Cancelling the SDK's caller does not release its stored
                        // RPC continuation. Disconnect to wake it on every exit.
                        let timedOut = (try? await Task.sleep(for: .seconds(180))) != nil
                        authorization.cancel()
                        await client.disconnect()
                        if timedOut { throw MCPFailure.connection }
                        throw CancellationError()
                    }
                    defer { group.cancelAll() }
                    guard let names = try await group.next() else { throw CancellationError() }
                    return names
                }
            } onCancel: {
                // Stop an active OAuth callback synchronously, including on app quit.
                authorization.cancel()
            }
        } catch {
            authorization.cancel()
            await client.disconnect()
            await authorization.waitForAuthorization()
            if authorization.authorizationFailure == .listenerShutdownTimeout { throw MCPFailure.listenerShutdownTimeout }
            try Task.checkCancellation()
            throw authorization.authorizationFailure ?? (error as? MCPFailure ?? .connection)
        }
    }

    func remove(_ endpoint: URL, retainingRegistration: Bool = false) throws {
        try store.attempts.begin(endpoint)
        defer { store.attempts.end(endpoint) }
        do {
            // A refused add must leave no grant in the Keychain. Keep the
            // registered client in memory so another add can sign in again.
            let registration = retainingRegistration ? try store.load(endpoint)?.registration : nil
            try store.delete(endpoint)
            store.attempts.remember(registration, for: endpoint)
        } catch { throw MCPFailure.credentials }
    }

    private static func list(_ client: Client, over transport: HTTPClientTransport, endpoint: URL) async throws -> [String] {
        try Task.checkCancellation()
        _ = try await client.connect(transport: transport)
        var names: [String] = []
        var known: Set<String> = []
        var cursors: Set<String> = []
        var cursor: String?
        for _ in 0..<32 {
            try Task.checkCancellation()
            let page = try await client.listTools(cursor: cursor)
            guard page.tools.count <= maxTools - names.count else { throw MCPFailure.toolLimit }
            for tool in page.tools {
                guard validToolName(tool.name) else { throw MCPFailure.toolLimit }
                guard known.insert(tool.name).inserted else { throw MCPFailure.duplicateTool(tool.name, endpoint, endpoint) }
                names.append(tool.name)
            }
            guard let next = page.nextCursor else { return names }
            guard next.utf8.prefix(4097).count <= 4096, cursors.insert(next).inserted else { throw MCPFailure.toolLimit }
            cursor = next
        }
        throw MCPFailure.toolLimit
    }

    static func validToolName(_ name: String) -> Bool {
        !name.isEmpty && name.utf8.prefix(maxToolNameBytes + 1).count <= maxToolNameBytes &&
            !name.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
    }
}
