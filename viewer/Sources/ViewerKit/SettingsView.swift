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

/// The viewer's settings window.
public struct SettingsView: View {
    @AppStorage(CorpusSettings.folderKey) private var folder = ""
    @State private var choosing = false

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
        }
        .padding(20)
        .frame(width: 560)
        .fileImporter(isPresented: $choosing, allowedContentTypes: [.folder]) { result in
            if case let .success(url) = result { folder = url.path }
        }
    }
}
