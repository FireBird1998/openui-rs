import SwiftUI

/// Where the viewer reads the corpus from.
public enum CorpusSettings {
    /// A folder path; empty means the conformance folder of the repository the
    /// viewer was built from.
    public static let folderKey = "conformanceFolder"

    public static var directory: URL {
        let folder = UserDefaults.standard.string(forKey: folderKey) ?? ""
        return folder.isEmpty ? Corpus.defaultDirectory : URL(fileURLWithPath: folder, isDirectory: true)
    }
}

/// The viewer's settings window: the corpus and recordings folders, the API key and the budget.
public struct SettingsView: View {
    @AppStorage(CorpusSettings.folderKey) private var folder = ""
    @AppStorage(Budget.capKey) private var cap = Budget.defaultCap
    @AppStorage(Budget.spentKey) private var spent = 0.0
    @AppStorage(Recordings.folderKey) private var recordingsFolder = ""
    @State private var choosing = false
    @State private var choosingRecordings = false
    @State private var newKey = ""
    @State private var hasKey = APIKeyStore.hasKey
    @State private var keyMessage: String?

    public init() {}

    public var body: some View {
        Form {
            LabeledContent("Corpus folder") {
                VStack(alignment: .leading, spacing: 4) {
                    Text(folder.isEmpty ? Corpus.defaultDirectory.path : folder)
                        .font(.callout.monospaced())
                        .textSelection(.enabled)
                    Text(folder.isEmpty
                         ? "The conformance folder of the repository this viewer was built from."
                         : "A conformance folder: its manifests, cases, schemas and goldens.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    HStack {
                        Button("Choose…") { choosing = true }
                        Button("Use the Built-in Folder") { folder = "" }
                            .disabled(folder.isEmpty)
                    }
                }
            }
            LabeledContent("Recordings folder") {
                VStack(alignment: .leading, spacing: 4) {
                    Text(Recordings.directory.path)
                        .font(.callout.monospaced())
                        .textSelection(.enabled)
                    HStack {
                        Button("Choose…") { choosingRecordings = true }
                        Button("Use the Default") { recordingsFolder = "" }
                            .disabled(recordingsFolder.isEmpty)
                        Button("Show in Finder") {
                            try? FileManager.default.createDirectory(at: Recordings.directory, withIntermediateDirectories: true)
                            Recordings.reveal(Recordings.directory)
                        }
                    }
                }
            }
            .fileImporter(isPresented: $choosingRecordings, allowedContentTypes: [.folder]) { result in
                if case let .success(url) = result { recordingsFolder = url.path }
            }
            Divider()
            LabeledContent("Anthropic API key") {
                VStack(alignment: .leading, spacing: 4) {
                    // The key goes straight into the Keychain and is never shown again.
                    SecureField(hasKey ? "A key is saved; enter a new one to replace it" : "sk-ant-…", text: $newKey)
                        .textContentType(.password)
                    HStack {
                        Button("Save to Keychain") {
                            do {
                                try APIKeyStore.save(newKey)
                                newKey = ""
                                keyMessage = "Saved."
                            } catch {
                                keyMessage = String(describing: error)
                            }
                            hasKey = APIKeyStore.hasKey
                        }
                        .disabled(newKey.trimmingCharacters(in: .whitespaces).isEmpty)
                        Button("Remove") {
                            do {
                                try APIKeyStore.delete()
                                keyMessage = "Removed."
                            } catch {
                                keyMessage = String(describing: error)
                            }
                            hasKey = APIKeyStore.hasKey
                        }
                        .disabled(!hasKey)
                    }
                    Text(keyMessage ?? (hasKey ? "A key is saved in your Keychain." : "No key saved."))
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            LabeledContent("Budget cap") {
                VStack(alignment: .leading, spacing: 4) {
                    TextField("Dollars", value: $cap, format: .currency(code: "USD"))
                        .frame(width: 120)
                    HStack {
                        Text("Spent so far (estimated): \(Budget.format(spent))")
                        Button("Reset") { spent = 0 }
                    }
                    Text("New requests stop once the estimated spending reaches the cap.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .padding(20)
        .frame(width: 560)
        .fileImporter(isPresented: $choosing, allowedContentTypes: [.folder]) { result in
            if case let .success(url) = result { folder = url.path }
        }
    }
}
