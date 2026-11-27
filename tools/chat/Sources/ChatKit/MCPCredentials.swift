import CryptoKit
import Foundation
import MCP
import OpenUINative
import Synchronization

struct MCPRegistration: Codable, Equatable, Sendable {
    let issuer: URL
    var tokenEndpoint: URL
    let redirect: URL
    let clientID: String
    let method: String
    let clientSecret: String?
    let secretExpiry: Date?
}

struct MCPLogin: Codable, Sendable {
    let registration: MCPRegistration
    var token: OAuthAccessToken?
}

protocol MCPCredentialStore: Sendable {
    var attempts: MCPLoginAttempts { get }
    func load(_ server: URL) throws -> MCPLogin?
    func save(_ login: MCPLogin, for server: URL) throws
    func delete(_ server: URL) throws
}

/// Serializes credential changes by server across services sharing the store.
/// Holds only registrations that have yet to reach the Keychain.
final class MCPLoginAttempts: Sendable {
    private struct State {
        var busy: Set<URL> = []
        var pending: [URL: MCPRegistration] = [:]
    }
    private let state = Mutex(State())

    func begin(_ server: URL) throws {
        guard state.withLock({ $0.busy.insert(server).inserted }) else { throw MCPFailure.busy }
    }

    func end(_ server: URL) {
        _ = state.withLock { $0.busy.remove(server) }
    }

    func registration(for server: URL) -> MCPRegistration? {
        state.withLock { $0.pending[server] }
    }

    func remember(_ registration: MCPRegistration?, for server: URL) {
        state.withLock { $0.pending[server] = registration }
    }
}

struct KeychainMCPStore: MCPCredentialStore {
    private static let sharedAttempts = MCPLoginAttempts()
    var attempts: MCPLoginAttempts { Self.sharedAttempts }

    func load(_ server: URL) throws -> MCPLogin? {
        guard let data = try item(server).read() else { return nil }
        guard data.count <= defaultHostByteLimit else { throw MCPFailure.credentials }
        return try JSONDecoder().decode(MCPLogin.self, from: data)
    }

    func save(_ login: MCPLogin, for server: URL) throws {
        let data = try JSONEncoder().encode(login)
        guard data.count <= defaultHostByteLimit else { throw MCPFailure.credentials }
        try item(server).save(data)
    }

    func delete(_ server: URL) throws { try item(server).delete() }

    private func item(_ server: URL) -> KeychainItem {
        let address = SHA256.hash(data: Data(server.absoluteString.utf8))
        return KeychainItem(account: "mcp-" + Data(address).base64EncodedString())
    }
}
