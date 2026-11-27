import Foundation
import Observation
import OpenUINative

@MainActor @Observable
final class MCPSettings {
    struct Tool: Codable, Identifiable {
        let name: String
        var enabled = true
        var id: String { name }
    }

    struct Server: Codable, Identifiable {
        let address: URL
        var tools: [Tool]
        var id: URL { address }
    }

    private static let preferenceKey = "mcpServers"
    nonisolated static let maxServers = 16
    private static let maxPreferenceBytes = defaultHostByteLimit
    @ObservationIgnored private let service: MCPService
    @ObservationIgnored private let preferences: any ChatPreferenceStore

    var address = ""
    private(set) var servers: [Server] = []
    private(set) var working = false
    private(set) var message: String?
    private var loadTask: Task<[String], any Error>?

    var canCancel: Bool { loadTask != nil }

    init(service: MCPService = MCPService(), preferences: any ChatPreferenceStore = UserDefaults.standard) {
        self.service = service
        self.preferences = preferences
        if let text = preferences.string(forKey: Self.preferenceKey),
           text.utf8.prefix(Self.maxPreferenceBytes + 1).count <= Self.maxPreferenceBytes,
           let saved = try? JSONDecoder().decode([Server].self, from: Data(text.utf8)) {
            var addresses: Set<URL> = []
            var names: Set<String> = []
            for server in saved.prefix(Self.maxServers) {
                guard MCPAddress.valid(server.address), !addresses.contains(server.address),
                      server.tools.count <= MCPService.maxTools,
                      server.tools.allSatisfy({ MCPService.validToolName($0.name) }) else { continue }
                let tools = Set(server.tools.map(\.name))
                guard tools.count == server.tools.count, names.isDisjoint(with: tools) else { continue }
                addresses.insert(server.address)
                names.formUnion(tools)
                servers.append(server)
            }
        }
    }

    func addServer() async {
        guard !working else { return }
        guard let endpoint = MCPAddress.parse(address) else {
            message = MCPFailure.address.localizedDescription
            return
        }
        guard !servers.contains(where: { $0.address == endpoint }) else {
            message = MCPFailure.configured.localizedDescription
            return
        }
        guard servers.count < Self.maxServers else {
            message = MCPFailure.serverLimit.localizedDescription
            return
        }
        let submittedAddress = address
        if await load(endpoint, adding: true), address == submittedAddress { address = "" }
    }

    func refresh(_ endpoint: URL) async {
        _ = await load(endpoint)
    }

    func signInAgain(_ endpoint: URL) async {
        _ = await load(endpoint, signInAgain: true)
    }

    func cancel() { loadTask?.cancel() }

    private func load(_ endpoint: URL, adding: Bool = false, signInAgain: Bool = false) async -> Bool {
        guard !working, adding || servers.contains(where: { $0.address == endpoint }) else { return false }
        working = true
        message = nil
        let task = Task {
            try await service.tools(at: endpoint, interactive: adding || signInAgain, signInAgain: signInAgain)
        }
        loadTask = task
        defer {
            loadTask = nil
            working = false
        }
        var listed = false
        do {
            let names = try await withTaskCancellationHandler {
                try await task.value
            } onCancel: {
                task.cancel()
            }
            listed = true
            guard !task.isCancelled, !Task.isCancelled else { throw CancellationError() }
            var owners: [String: URL] = [:]
            for server in servers where server.address != endpoint {
                for tool in server.tools { owners[tool.name] = server.address }
            }
            for name in names {
                if let owner = owners[name] { throw MCPFailure.duplicateTool(name, owner, endpoint) }
            }
            let previous = servers.first(where: { $0.address == endpoint })
            let disabled = Set(previous?.tools.filter { !$0.enabled }.map(\.name) ?? [])
            let fresh = Server(address: endpoint, tools: names.map { Tool(name: $0, enabled: !disabled.contains($0)) })
            if adding {
                servers.append(fresh)
            } else {
                servers = servers.map { $0.address == endpoint ? fresh : $0 }
            }
            persist()
            return true
        } catch {
            if adding && listed {
                do { try await service.remove(endpoint, retainingRegistration: true) }
                catch {
                    message = MCPFailure.credentials.localizedDescription
                    return false
                }
            }
            if !(error is CancellationError) {
                message = (error as? MCPFailure ?? .connection).localizedDescription
            }
            return false
        }
    }

    func setEnabled(_ enabled: Bool, server endpoint: URL, tool name: String) {
        guard !working, var server = servers.first(where: { $0.address == endpoint }) else { return }
        server.tools = server.tools.map { $0.name == name ? Tool(name: name, enabled: enabled) : $0 }
        servers = servers.map { $0.address == endpoint ? server : $0 }
        persist()
    }

    func remove(_ endpoint: URL) async {
        guard !working, servers.contains(where: { $0.address == endpoint }) else { return }
        working = true
        message = nil
        defer { working = false }
        do {
            try await service.remove(endpoint)
            servers.removeAll { $0.address == endpoint }
            persist()
        } catch {
            message = (error as? MCPFailure ?? .credentials).localizedDescription
        }
    }

    private func persist() {
        guard let data = try? JSONEncoder().encode(servers), data.count <= Self.maxPreferenceBytes else { return }
        preferences.set(String(decoding: data, as: UTF8.self), forKey: Self.preferenceKey)
    }
}
