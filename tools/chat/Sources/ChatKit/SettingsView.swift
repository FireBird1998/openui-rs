import SwiftUI

/// Settings: the Thesys API key, typed here and kept in the Keychain.
public struct SettingsView: View {
    @State private var settings: KeySettings

    public init(settings: KeySettings = KeySettings(store: KeychainKeyStore())) {
        _settings = State(initialValue: settings)
    }

    public var body: some View {
        Form {
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
        .formStyle(.grouped)
        .frame(width: 440)
    }
}
