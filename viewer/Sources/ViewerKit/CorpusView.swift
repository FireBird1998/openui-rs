import Observation
import SwiftUI

/// The viewer's window: the corpus on the left, the open program played in
/// the middle, and the inspector on the right.
public struct CorpusView: View {
    @State private var model = CorpusModel()
    @State private var search = ""
    @State private var showInspector = true
    @State private var showNewProgram = false
    @AppStorage(CorpusSettings.folderKey) private var folder = ""

    public init() {}

    public var body: some View {
        NavigationSplitView {
            List(selection: $model.selected) {
                ForEach(model.sections(matching: search)) { section in
                    Section(section.title) {
                        ForEach(section.cases) { c in
                            Text(c.name).tag(c.name)
                        }
                    }
                }
            }
            .searchable(text: $search, placement: .sidebar, prompt: "Find a case")
            .navigationSplitViewColumnWidth(min: 220, ideal: 280)
        } detail: {
            detail
        }
        .navigationTitle(model.current?.title ?? "Viewer")
        .toolbar {
            ToolbarItem {
                Button("Open a Program", systemImage: "doc.badge.plus") { showNewProgram = true }
                    .help("Paste a program or open a file, with a component library")
            }
            ToolbarItem {
                Button("Inspector", systemImage: "sidebar.right") { showInspector.toggle() }
            }
        }
        .sheet(isPresented: $showNewProgram) {
            NewProgramSheet(schemaNames: model.schemaNames) { request in
                Task { await model.open(request) }
            }
        }
        // Opens the case after the list has finished updating its selection.
        .task(id: model.selected) { await model.openSelected() }
        .onChange(of: model.chunking) { Task { await model.rechunk() } }
        .onChange(of: folder) { model.reloadCorpus() }
        .onReceive(NotificationCenter.default.publisher(for: Gallery.openNotification)) { _ in
            Task { await model.open(Gallery.request) }
        }
        .task {
            if let request = ScriptedRun.request { await ScriptedRun.run(request, model: model) }
        }
        .environment(model.selection)
    }

    @ViewBuilder private var detail: some View {
        if let message = model.failure {
            ContentUnavailableView("Couldn't open it", systemImage: "exclamationmark.triangle", description: Text(message))
        } else if let playback = model.playback, let current = model.current {
            PlaybackView(
                playback: playback,
                chunking: $model.chunking,
                chunkings: model.chunkings,
                program: current.text
            )
            .inspector(isPresented: $showInspector) {
                InspectorView(playback: playback, golden: current.golden, chunking: model.chunking)
                    .inspectorColumnWidth(min: 260, ideal: 340, max: 600)
            }
        } else {
            ContentUnavailableView("Pick a case", systemImage: "square.dashed",
                                   description: Text("Or open a program of your own with the toolbar's + button."))
        }
    }
}

/// The program on screen and where it came from.
struct OpenProgram {
    var title: String
    var text: String
    /// The reference's step fingerprints, for corpus cases.
    var golden: GoldenSteps?
}

@MainActor @Observable
final class CorpusModel {
    private(set) var corpus: Corpus?
    var selected: String?
    var chunking = Chunking.fixed(4)
    private(set) var current: OpenProgram?
    private(set) var playback: Playback?
    /// The chunking the playback was last loaded with.
    private var loadedChunking: Chunking?
    private(set) var failure: String?
    let selection = ElementSelection()
    private let core = Core()
    private var libraries: [String: Library] = [:]

    init() {
        reloadCorpus()
    }

    func reloadCorpus() {
        libraries = [:]
        do {
            corpus = try Corpus.load(from: CorpusSettings.directory)
            failure = nil
        } catch {
            corpus = nil
            failure = String(describing: error)
        }
    }

    func sections(matching search: String) -> [CorpusSection] {
        corpus?.sections(matching: search) ?? []
    }

    var schemaNames: [String] { corpus?.schemas.keys.sorted() ?? [] }

    /// The fixed chunkings, plus the corpus's random ones for a corpus case.
    var chunkings: [Chunking] {
        Chunking.fixedSizes.map(Chunking.fixed) + (current?.golden?.recordedChunkings ?? [])
    }

    func openSelected() async {
        guard let corpus, let name = selected, let c = corpus.cases.first(where: { $0.name == name }) else { return }
        do {
            let library = try await library(named: c.schemaName, in: corpus)
            await show(OpenProgram(title: c.name, text: try corpus.program(of: c), golden: corpus.golden(for: c)),
                       library: library, root: c.root)
        } catch {
            failure = String(describing: error)
        }
    }

    func open(_ request: ProgramRequest) async {
        selected = nil
        do {
            let library: Library
            switch request.library {
            case let .corpus(name):
                guard let corpus else { return }
                library = try await self.library(named: name, in: corpus)
            case let .file(url):
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                library = try await core.library(schema: String(contentsOf: url, encoding: .utf8))
            }
            await show(OpenProgram(title: "\(request.title) · \(request.library.label)", text: request.text, golden: nil),
                       library: library, root: request.root)
        } catch {
            failure = String(describing: error)
        }
    }

    private func library(named name: String, in corpus: Corpus) async throws -> Library {
        if let loaded = libraries[name] { return loaded }
        guard let path = corpus.schemas[name] else {
            throw DecodeError(description: "no schema named \(name)")
        }
        let schema = try String(contentsOf: corpus.directory.appendingPathComponent(path), encoding: .utf8)
        let library = try await core.library(schema: schema)
        libraries[name] = library
        return library
    }

    private func show(_ program: OpenProgram, library: Library, root: String?) async {
        failure = nil
        selection.clear()
        if case .recorded = chunking { chunking = .fixed(4) } // recorded sizes belong to one case
        current = program
        let playback = Playback(core: core, library: library, root: root)
        self.playback = playback
        loadedChunking = chunking
        await playback.load(chunking.source(for: program.text))
    }

    /// Cuts the same program the new way, staying near the same place.
    func rechunk() async {
        guard let playback, let current, chunking != loadedChunking else { return }
        loadedChunking = chunking
        await playback.load(chunking.source(for: current.text), resumeAt: playback.received)
    }
}
