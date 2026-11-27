import AppKit
import SwiftUI

/// Settings for the system message, the Thesys API key and MCP servers.
public struct SettingsView: View {
    @State private var settings: KeySettings
    @State private var mcp: MCPSettings
    private let preferences: ChatPreferences

    public init(settings: KeySettings = KeySettings(store: KeychainKeyStore()),
                preferences: ChatPreferences = ChatPreferences()) {
        self.init(settings: settings, preferences: preferences, mcp: MCPSettings())
    }

    init(settings: KeySettings, preferences: ChatPreferences, mcp: MCPSettings) {
        self.preferences = preferences
        _settings = State(initialValue: settings)
        _mcp = State(initialValue: mcp)
    }

    public var body: some View {
        @Bindable var preferences = preferences
        Form {
            Section {
                Picker("Prompt mode", selection: $preferences.promptMode) {
                    Text("Cloud prompt").tag(ChatPromptMode.cloud)
                    Text("Full prompt").tag(ChatPromptMode.full)
                }
            }
            Section {
                SecureField("Thesys API key", text: $settings.draft)
                    .onSubmit { settings.save() }
                HStack {
                    Button("Save") { settings.save() }
                        .keyboardShortcut(.defaultAction)
                    Button("Remove Key", role: .destructive) { settings.remove() }
                        .disabled(!settings.hasKey)
                }
                Text(settings.message ?? (settings.hasKey ? "A key is saved in the Keychain." : "No key is saved."))
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } footer: {
                Text("The key is kept in your login Keychain and sent only to api.thesys.dev.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            MCPSettingsSection(settings: mcp)
        }
        .background(SettingsWindowLifetime(settings: mcp))
        .preferredColorScheme(preferences.themeOverride)
        .formStyle(.grouped)
        .frame(minHeight: 460)
        .frame(width: 480)
    }
}

private struct SettingsWindowLifetime: NSViewRepresentable {
    let settings: MCPSettings

    func makeNSView(context: Context) -> Observer { Observer() }

    func updateNSView(_ observer: Observer, context: Context) { observer.settings = settings }

    final class Observer: NSView {
        weak var settings: MCPSettings?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            let center = NotificationCenter.default
            center.removeObserver(self)
            guard let window else { return }
            center.addObserver(self, selector: #selector(cancel), name: NSWindow.willCloseNotification, object: window)
            center.addObserver(self, selector: #selector(cancel), name: NSApplication.willTerminateNotification, object: NSApp)
        }

        @objc private func cancel(_ notification: Notification) { settings?.cancel() }
    }
}
