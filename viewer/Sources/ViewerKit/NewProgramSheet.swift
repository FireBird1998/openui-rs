import SwiftUI
import UniformTypeIdentifiers

/// A program from outside the corpus: pasted, or read from a file, with a
/// component library from the corpus or from a JSON Schema file.
struct ProgramRequest {
    var title: String
    var text: String
    var library: LibraryChoice
    var root: String?
}

enum LibraryChoice: Hashable {
    /// One of the corpus's schemas, by name.
    case corpus(String)
    /// A JSON Schema file.
    case file(URL)

    var label: String {
        switch self {
        case let .corpus(name): name
        case let .file(url): url.lastPathComponent
        }
    }
}

struct NewProgramSheet: View {
    let schemaNames: [String]
    let open: (ProgramRequest) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var title = "Pasted program"
    @State private var library: LibraryChoice
    @State private var schemaFiles: [URL] = []
    @State private var root = ""
    @State private var importing: Importing?
    @State private var failure: String?

    enum Importing: Identifiable {
        case program, schema
        var id: Self { self }
    }

    init(schemaNames: [String], open: @escaping (ProgramRequest) -> Void) {
        self.schemaNames = schemaNames
        self.open = open
        _library = State(initialValue: .corpus(schemaNames.contains("bench") ? "bench" : schemaNames.first ?? ""))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Open a program").font(.title2.bold())
            HStack {
                Text("Paste OpenUI Lang below, or")
                Button("Choose a File…") { importing = .program }
            }
            TextEditor(text: $text)
                .font(.body.monospaced())
                .frame(minHeight: 240)
                .border(.separator)
            Form {
                Picker("Component library", selection: $library) {
                    ForEach(schemaNames, id: \.self) { name in
                        Text(name).tag(LibraryChoice.corpus(name))
                    }
                    ForEach(schemaFiles, id: \.self) { url in
                        Text(url.lastPathComponent).tag(LibraryChoice.file(url))
                    }
                }
                Button("Choose a JSON Schema File…") { importing = .schema }
                TextField("Root name (optional)", text: $root)
            }
            if let failure {
                Text(failure).foregroundStyle(.red).font(.callout)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Open") {
                    open(ProgramRequest(title: title, text: text, library: library, root: root.isEmpty ? nil : root))
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(text.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 640)
        .fileImporter(
            isPresented: Binding(get: { importing != nil }, set: { if !$0 { importing = nil } }),
            allowedContentTypes: importing == .schema ? [.json] : [.plainText, .data]
        ) { result in
            let kind = importing
            importing = nil
            do {
                let url = try result.get()
                switch kind {
                case .program:
                    text = try read(url)
                    title = url.lastPathComponent
                case .schema:
                    if !schemaFiles.contains(url) { schemaFiles.append(url) }
                    library = .file(url)
                case nil:
                    break
                }
                failure = nil
            } catch {
                failure = String(describing: error)
            }
        }
    }

    private func read(_ url: URL) throws -> String {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        return try String(contentsOf: url, encoding: .utf8)
    }
}
