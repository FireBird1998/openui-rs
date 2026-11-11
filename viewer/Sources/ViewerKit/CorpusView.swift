import Observation
import SwiftUI

/// The viewer's window: the corpus on the left, the open program played in
/// the middle, and the inspector on the right.
public struct CorpusView: View {
    @State private var model = CorpusModel()
    @State private var search = ""
    @State private var showInspector = true
    @State private var showNewProgram = false
    @State private var showAskClaude = false
    @State private var showFollowUp = false
    @AppStorage(CorpusSettings.folderKey) private var folder = ""
    @AppStorage(Recordings.folderKey) private var recordingsFolder = ""

    public init() {}

    public var body: some View {
        NavigationSplitView {
            List(selection: $model.selected) {
                let recordings = model.recordings(matching: search)
                if !recordings.isEmpty {
                    Section("Recordings") {
                        ForEach(recordings) { file in
                            Text(file.title).tag(file.id)
                                .help(file.url.lastPathComponent)
                        }
                    }
                }
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
                Button("Ask Claude", systemImage: "sparkles") { showAskClaude = true }
                    .help("Ask Claude for a screen and watch it stream")
            }
            ToolbarItem {
                Button("Open a Program", systemImage: "doc.badge.plus") { showNewProgram = true }
                    .help("Paste a program or open a file, with a component library")
            }
            ToolbarItem {
                Button("Inspector", systemImage: "sidebar.right") { showInspector.toggle() }
            }
        }
        .sheet(isPresented: $showAskClaude) {
            AskClaudeSheet(libraries: model.promptLibraries) { draft in
                Task { await model.openLive(draft) }
            }
        }
        .sheet(isPresented: $showFollowUp) {
            AskClaudeSheet(libraries: model.promptLibraries, base: model.current?.finalProgram,
                           baseLibrary: model.current?.libraryName) { draft in
                Task { await model.openLive(draft) }
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
        .onChange(of: recordingsFolder) { model.reloadRecordings() }
        .onReceive(NotificationCenter.default.publisher(for: Gallery.openNotification)) { _ in
            Task { await model.open(Gallery.request) }
        }
        .task {
            if ProcessInfo.processInfo.environment["VIEWER_RECORD"] != nil {
                await BatchRecordRun.run(model: model)
            } else if let request = ScriptedRun.request {
                await ScriptedRun.run(request, model: model)
            }
        }
        .disabled(ProcessInfo.processInfo.environment["VIEWER_RECORD"] != nil)
        .environment(model.selection)
    }

    @ViewBuilder private var detail: some View {
        if let message = model.failure {
            ContentUnavailableView("Couldn't open it", systemImage: "exclamationmark.triangle", description: Text(message))
        } else if let playback = model.playback, let current = model.current {
            VStack(spacing: 0) {
                if let live = model.liveStatus {
                    LiveBar(status: live, stop: { model.stopLive() }, save: { model.saveRecording() },
                            saved: model.savedRecording, followUp: model.canFollowUp ? { showFollowUp = true } : nil)
                        .padding(.horizontal).padding(.vertical, 8)
                    Divider()
                }
                if let recording = current.recording {
                    RecordingBar(file: current.recordingFile, recording: recording,
                                 replay: { Task { await model.replayRecording() } },
                                 followUp: model.canFollowUp ? { showFollowUp = true } : nil)
                    .padding(.horizontal).padding(.vertical, 8)
                    Divider()
                }
                playbackView(playback, current: current)
            }
        } else {
            ContentUnavailableView("Pick a case", systemImage: "square.dashed",
                                   description: Text("Or open a program of your own with the toolbar's + button."))
        }
    }

    private func playbackView(_ playback: Playback, current: OpenProgram) -> some View {
            PlaybackView(
                playback: playback,
                chunking: $model.chunking,
                chunkings: model.chunkings,
                live: model.liveStatus
            )
            .inspector(isPresented: $showInspector) {
                InspectorView(playback: playback, golden: current.golden, chunking: model.chunking)
                    .inspectorColumnWidth(min: 260, ideal: 340, max: 600)
            }
    }
}

/// The program on screen and where it came from.
struct OpenProgram {
    var title: String
    /// The whole program; for a live request, filled in once it has all arrived.
    var text: String
    /// The reference's step fingerprints, for corpus cases.
    var golden: GoldenSteps?
    /// The recording it was opened from, and its file.
    var recording: Recording?
    var recordingFile: URL?
    /// For an edit-mode patch, the program it patches.
    var base: String?
    /// The component library's name in the corpus, if it has one.
    var libraryName: String?
    /// The whole program once it has all arrived: for a patch, the merged result.
    var finalProgram: String?
}

@MainActor @Observable
final class CorpusModel {
    private(set) var corpus: Corpus?
    var selected: String?
    var chunking = Chunking.fixed(4)
    private(set) var current: OpenProgram?
    private(set) var playback: Playback?
    /// The live request on screen, if the program came from one.
    private(set) var liveStatus: LiveStatus?
    /// What it asked for and when, for saving it as a recording.
    private var liveDraft: LiveDraft?
    private var liveStarted = Date()
    /// Where the live request on screen was saved, once it has been.
    private(set) var savedRecording: URL?
    private(set) var recordings: [RecordingFile] = []
    /// The chunking the playback was last loaded with.
    private var loadedChunking: Chunking?
    private(set) var failure: String?
    let selection = ElementSelection()
    private let core = Core()
    private var libraries: [String: Library] = [:]

    init(corpus: Corpus? = nil) {
        if let corpus { self.corpus = corpus } else { reloadCorpus() }
        reloadRecordings()
    }

    func reloadRecordings() {
        recordings = Recordings.list()
    }

    func recordings(matching search: String) -> [RecordingFile] {
        recordings.filter {
            search.isEmpty || $0.title.localizedCaseInsensitiveContains(search)
                || $0.url.lastPathComponent.localizedCaseInsensitiveContains(search)
        }
    }

    /// Saves the live request on screen as a recording.
    func saveRecording() {
        do { _ = try saveRecording(batchRequest: nil, in: Recordings.directory) }
        catch { failure = String(describing: error) }
    }

    @discardableResult
    func saveRecording(batchRequest: BatchRequest?, in directory: URL) throws -> URL {
        guard let status = liveStatus, let draft = liveDraft, !status.isActive else {
            throw ClaudeError(description: "No finished stream to save")
        }
        var recording = Recording(status: status, request: draft.text, effort: draft.effort,
                                  library: draft.library, createdAt: liveStarted, patchOf: draft.base)
        recording.batchRequest = batchRequest
        let url = try Recordings.save(recording, in: directory)
        savedRecording = url
        reloadRecordings()
        return url
    }

    /// Opens a saved recording, cut where its deltas arrived.
    func openRecording(_ file: RecordingFile) async {
        guard let corpus else { return }
        do {
            let library = try await library(named: file.recording.library, in: corpus)
            let program = OpenProgram(title: "Recording · \(file.title)", text: file.recording.text, golden: nil,
                                      recording: file.recording, recordingFile: file.url,
                                      base: file.recording.patchOf, libraryName: file.recording.library)
            await show(program, library: library, root: nil,
                       source: RecordingSource(recording: file.recording, timed: false),
                       chunking: file.recording.chunking, base: file.recording.patchOf)
        } catch {
            failure = String(describing: error)
        }
    }

    /// Plays the recording on screen again with the timing it arrived with.
    func replayRecording() async {
        guard let playback, let recording = current?.recording else { return }
        chunking = recording.chunking
        loadedChunking = chunking
        await playback.load(RecordingSource(recording: recording, timed: true))
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

    var promptLibraries: [String] { corpus?.promptLibraries ?? [] }

    /// The fixed chunkings, plus the corpus's random ones for a corpus case,
    /// or the received one for a recording.
    var chunkings: [Chunking] {
        Chunking.fixedSizes.map(Chunking.fixed) + (current?.golden?.recordedChunkings ?? [])
            + (current?.recording.map { [$0.chunking] } ?? [])
    }

    func openSelected() async {
        if let id = selected, let file = recordings.first(where: { $0.id == id }) {
            await openRecording(file)
            return
        }
        guard let corpus, let name = selected, let c = corpus.cases.first(where: { $0.name == name }) else { return }
        do {
            let library = try await library(named: c.schemaName, in: corpus)
            await show(OpenProgram(title: c.name, text: try corpus.program(of: c), golden: corpus.golden(for: c),
                                   libraryName: c.schemaName),
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

    /// Whether the screen can be changed with a follow-up: it came from a live
    /// request or a recording in a library with a prompt spec, and has arrived.
    var canFollowUp: Bool {
        guard let current, current.finalProgram != nil, let library = current.libraryName else { return false }
        return (liveStatus != nil || current.recording != nil) && promptLibraries.contains(library)
            && !(playback?.isStreaming ?? true)
    }

    struct PreparedLive {
        let draft: LiveDraft
        let library: Library
        let request: ClaudeRequest
    }

    /// Shared by the sheet and batch mode, including the library's edit prompt.
    func prepareLive(_ draft: LiveDraft) async throws -> PreparedLive {
        guard let corpus else { throw ClaudeError(description: "No corpus loaded") }
        guard let spec = corpus.promptSpec(forLibrary: draft.library, editMode: draft.base != nil) else {
            throw ClaudeError(description: "The corpus has no prompt spec for \(draft.library)")
        }
        let library = try await library(named: draft.library, in: corpus)
        let systemPrompt = try await core.systemPrompt(spec: spec)
        let userText = draft.base.map { ClaudeRequest.followUp(draft.text, to: $0) } ?? draft.text
        return PreparedLive(draft: draft, library: library,
                            request: ClaudeRequest(model: draft.model, effort: draft.effort,
                                                   systemPrompt: systemPrompt, userText: userText))
    }

    /// The only live source factory: both modes use the same Keychain path.
    /// Tests inject a fake source instead and never call this method.
    static func liveSource(request: ClaudeRequest, status: LiveStatus) throws -> any ChunkSource {
        guard let key = APIKeyStore.read(), !key.isEmpty else {
            throw ClaudeError(description: "No API key in the Keychain. Add one in Settings.")
        }
        return ClaudeSource(request: request, apiKey: key, status: status)
    }

    /// The batch's source factory: the same Keychain path, read once for
    /// the run and held in memory only. A rebuilt viewer has a new signature,
    /// so the Keychain asks the maintainer to allow each read; one answer
    /// then covers all the batch's requests.
    @MainActor
    static func batchSource() -> BatchRecorder.SourceFactory {
        var key: String?
        return { request, status in
            if key == nil { key = APIKeyStore.read() }
            guard let key, !key.isEmpty else {
                throw ClaudeError(description: "No API key in the Keychain. Add one in Settings.")
            }
            return ClaudeSource(request: request, apiKey: key, status: status)
        }
    }

    func streamLive(_ prepared: PreparedLive, status: LiveStatus, source: any ChunkSource) async {
        let draft = prepared.draft
        selected = nil
        let title = (draft.base == nil ? "Claude · " : "Claude · change: ")
            + draft.text.prefix(48).replacingOccurrences(of: "\n", with: " ")
        liveDraft = draft
        liveStarted = Date()
        await show(OpenProgram(title: title, text: "", golden: nil, base: draft.base, libraryName: draft.library),
                   library: prepared.library, root: nil, source: source, live: status, base: draft.base)
        if let playback { current?.text = playback.text }
    }

    /// Asks from the sheet and plays the answer as it streams.
    func openLive(_ draft: LiveDraft) async {
        guard !Budget.reached else {
            failure = "The budget cap (\(Budget.format(Budget.cap))) is reached. Raise it or reset the spending in Settings."
            return
        }
        do {
            let prepared = try await prepareLive(draft)
            let status = LiveStatus(model: draft.model)
            let source = try Self.liveSource(request: prepared.request, status: status)
            await streamLive(prepared, status: status, source: source)
            Budget.add(status.cost)
        } catch {
            failure = String(describing: error)
        }
    }

    /// Stops the live request on screen, keeping what has arrived.
    func stopLive() {
        playback?.stop()
    }

    private func show(_ program: OpenProgram, library: Library, root: String?,
                      source: (any ChunkSource)? = nil, live: LiveStatus? = nil, chunking initial: Chunking? = nil,
                      base: String? = nil) async {
        failure = nil
        selection.clear()
        if let initial {
            chunking = initial
        } else if case .recorded = chunking {
            chunking = .fixed(4) // recorded sizes belong to one case
        }
        current = program
        liveStatus = live
        savedRecording = nil
        let playback = Playback(core: core, library: library, root: root, base: base)
        self.playback = playback
        loadedChunking = chunking
        if let source {
            await playback.load(source)
        } else {
            await playback.load(chunking.source(for: program.text))
        }
        // Shown last: the whole program, or for a patch the merged result.
        if self.playback === playback {
            current?.finalProgram = playback.step?.program ?? playback.text
        }
    }

    /// Cuts the same program the new way, staying near the same place.
    func rechunk() async {
        guard let playback, let current, chunking != loadedChunking, !playback.isStreaming else { return }
        loadedChunking = chunking
        await playback.load(chunking.source(for: current.text), resumeAt: playback.received)
    }
}
