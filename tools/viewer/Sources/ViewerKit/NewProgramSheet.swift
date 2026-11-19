import OpenUINative
import SwiftUI
import UniformTypeIdentifiers

/// A program from outside the corpus: pasted, or read from a file, with a
/// component library from the corpus or from a JSON Schema file.
struct ProgramRequest {
    var title: String
    var text: String
    var library: LibraryChoice
    var root: String?
    var toolScript: ToolScript?

    static func newsDesk() throws -> Self {
        guard let program = Bundle.module.url(forResource: "news-desk", withExtension: "oui", subdirectory: "Samples"),
              let script = Bundle.module.url(forResource: "news-desk.tools", withExtension: "json", subdirectory: "Samples")
        else { throw DecodeError(description: "Missing News Desk sample files") }
        return try ProgramRequest(title: "News desk", text: String(contentsOf: program, encoding: .utf8),
                                  library: .corpus("bench"), root: nil, toolScript: ToolScript(contentsOf: script))
    }
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
    @State private var importing = Importing.program
    @State private var showImporter = false
    @State private var scriptFile: (name: String, script: ToolScript)?
    @State private var failure: String?

    enum Importing {
        case program, schema, script
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
                Button("Choose a File…") { choose(.program) }
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
                Button("Choose a JSON Schema File…") { choose(.schema) }
                HStack {
                    Button("Choose a Tool Script…") { choose(.script) }
                    if let scriptFile {
                        Text(scriptFile.name)
                        Button("Remove") { self.scriptFile = nil }
                    }
                }
                TextField("Root name (optional)", text: $root)
            }
            if let failure {
                Text(failure).foregroundStyle(.red).font(.callout)
            }
            HStack {
                Button("Open News Desk Sample") { openSample { try .newsDesk() } }
                    .disabled(!schemaNames.contains("bench"))
                Menu("Response Samples") {
                    Button("Message") { openSample { try .responseSample(bundle: false) } }
                    Button("Response Bundle") { openSample { try .responseSample(bundle: true) } }
                }
                .disabled(!schemaNames.contains("bench"))
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Open") {
                    open(ProgramRequest(title: title, text: text, library: library, root: root.isEmpty ? nil : root,
                                        toolScript: scriptFile?.script))
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(text.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 640)
        .fileImporter(
            isPresented: $showImporter,
            allowedContentTypes: importing == .program ? [.plainText, .data] : [.json]
        ) { result in
            do {
                let url = try result.get()
                switch importing {
                case .program:
                    text = try read(url)
                    title = url.lastPathComponent
                case .schema:
                    if !schemaFiles.contains(url) { schemaFiles.append(url) }
                    library = .file(url)
                case .script:
                    scriptFile = (url.lastPathComponent, try ToolScript(contentsOf: url))
                }
                failure = nil
            } catch {
                failure = String(describing: error)
            }
        }
    }

    private func openSample(_ load: () throws -> ProgramRequest) {
        do {
            open(try load())
            dismiss()
        } catch { failure = String(describing: error) }
    }

    private func choose(_ kind: Importing) {
        importing = kind
        showImporter = true
    }

    private func read(_ url: URL) throws -> String {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        return try String(contentsOf: url, encoding: .utf8)
    }
}
