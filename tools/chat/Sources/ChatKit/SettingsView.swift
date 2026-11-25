import SwiftUI

/// Settings for the system message and the Keychain-backed Thesys API key.
public struct SettingsView: View {
    @State private var settings: KeySettings
    private let preferences: ChatPreferences

    public init(settings: KeySettings = KeySettings(store: KeychainKeyStore()),
                preferences: ChatPreferences = ChatPreferences()) {
        self.preferences = preferences
        _settings = State(initialValue: settings)
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
        }
        .preferredColorScheme(preferences.themeOverride)
        .formStyle(.grouped)
        .frame(width: 440)
    }
}
