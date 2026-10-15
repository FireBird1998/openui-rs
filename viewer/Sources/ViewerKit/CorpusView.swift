import Observation
import SwiftUI

/// The viewer's window: corpus cases on the left, the chosen one drawn on the right.
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
        .task(id: model.selected) { model.open() }
    }

    @ViewBuilder private var detail: some View {
        switch model.state {
        case .empty:
            ContentUnavailableView("Pick a case", systemImage: "square.dashed")
        case let .failed(message):
            ContentUnavailableView("Couldn't open it", systemImage: "exclamationmark.triangle", description: Text(message))
        case let .parsed(result):
            if let root = result.root {
                ScrollView([.vertical, .horizontal]) {
                    ElementBox(element: root).padding()
                }
            } else {
                ContentUnavailableView("No root element", systemImage: "square.dashed")
            }
        }
    }
}

@MainActor @Observable
final class CorpusModel {
    enum State {
        case empty
        case failed(String)
        case parsed(ParseResult)
    }

    private(set) var cases: [CorpusCase] = []
    var selected: String?
    private(set) var state = State.empty
    private var corpus: Corpus?
    private var libraries: [String: Library] = [:]

    init() {
        do {
            let corpus = try Corpus.load()
            self.corpus = corpus
            cases = corpus.cases
        } catch {
            state = .failed(String(describing: error))
        }
    }

    func open() {
        guard let corpus, let name = selected, let c = cases.first(where: { $0.name == name }) else { return }
        do {
            let library = try libraries[c.schemaName] ?? Library(schema: corpus.schema(of: c))
            libraries[c.schemaName] = library
            state = .parsed(try parseOneShot(corpus.program(of: c), library: library, root: c.root))
        } catch {
            state = .failed(String(describing: error))
        }
    }
}
