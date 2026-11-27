import SwiftUI

struct MCPSettingsSection: View {
    @Bindable var settings: MCPSettings

    var body: some View {
        Section("MCP Servers") {
            HStack {
                TextField("Server address", text: $settings.address, prompt: Text("https://mcp.notion.com/mcp"))
                    .onSubmit { Task { await settings.addServer() } }
                    .disabled(settings.working)
                Button("Connect") { Task { await settings.addServer() } }
                    .disabled(settings.working || settings.address.isEmpty)
                if settings.working {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityLabel("MCP server request in progress")
                }
                if settings.canCancel {
                    Button("Cancel") { settings.cancel() }
                        .buttonStyle(.borderless)
                }
            }
            if let message = settings.message {
                Text(message).font(.callout).foregroundStyle(.red).textSelection(.enabled)
            }
            ForEach(settings.servers) { server in
                DisclosureGroup {
                    if server.tools.isEmpty {
                        Text("This server has no tools.").foregroundStyle(.secondary)
                    }
                    ForEach(server.tools) { tool in
                        Toggle(isOn: Binding(
                            get: { tool.enabled },
                            set: { settings.setEnabled($0, server: server.address, tool: tool.name) }
                        )) {
                            Text(tool.name)
                        }
                        .toggleStyle(.switch)
                    }
                    HStack {
                        Button("Refresh Tools") { Task { await settings.refresh(server.address) } }
                        Button("Sign In Again") { Task { await settings.signInAgain(server.address) } }
                        Button("Remove", role: .destructive) { Task { await settings.remove(server.address) } }
                    }
                } label: {
                    Text(server.address.absoluteString)
                }
                .disabled(settings.working)
            }
        }
    }
}
