import Observation
import SwiftUI

/// The viewer's window: corpus cases on the left, the chosen one played on the right.
public struct CorpusView: View {
    @State private var model = CorpusModel()

    public init() {}

    public var body: some View {
        NavigationSplitView {
            List(model.cases, selection: $model.selected) { c in
                Text(c.name).tag(c.name)
            }
            .navigationSplitViewColumnWidth(min: 220, ideal: 280)
        } detail: {
            detail
        }
        // Opens the case after the list has finished updating its selection.
        .task(id: model.selected) { await model.open() }
        .onChange(of: model.chunkSize) { Task { await model.rechunk() } }
        .task {
            if let request = ScriptedRun.request { await ScriptedRun.run(request, model: model) }
        }
    }

    @ViewBuilder private var detail: some View {
        if let message = model.failure {
            ContentUnavailableView("Couldn't open it", systemImage: "exclamationmark.triangle", description: Text(message))
        } else if let playback = model.playback {
            PlaybackView(playback: playback, chunkSize: $model.chunkSize)
        } else {
            ContentUnavailableView("Pick a case", systemImage: "square.dashed")
        }
    }
}

@MainActor @Observable
final class CorpusModel {
    private(set) var cases: [CorpusCase] = []
    var selected: String?
    var chunkSize = 4
    private(set) var playback: Playback?
    private(set) var failure: String?
    private let core = Core()
    private var corpus: Corpus?
    private var libraries: [String: Library] = [:]
    private var program = ""

    init() {
        do {
            let corpus = try Corpus.load()
            self.corpus = corpus
            cases = corpus.cases
        } catch {
            failure = String(describing: error)
        }
    }

    func open() async {
        guard let corpus, let name = selected, let c = cases.first(where: { $0.name == name }) else { return }
        do {
            let library: Library
            if let loaded = libraries[c.schemaName] {
                library = loaded
            } else {
                library = try await core.library(schema: corpus.schema(of: c))
                libraries[c.schemaName] = library
            }
            program = try corpus.program(of: c)
            failure = nil
            let playback = Playback(core: core, library: library, root: c.root)
            self.playback = playback
            await playback.load(ReplaySource(program: program, chunkSize: chunkSize))
        } catch {
            failure = String(describing: error)
        }
    }

    /// Cuts the same program at the new chunk size, staying near the same place.
    func rechunk() async {
        guard let playback else { return }
        await playback.load(ReplaySource(program: program, chunkSize: chunkSize), resumeAt: playback.received)
    }
}
