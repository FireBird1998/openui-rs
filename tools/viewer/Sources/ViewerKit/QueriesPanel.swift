import OpenUINative
import SwiftUI

struct QueriesPanel: View {
    let live: LiveProgram
    @State private var retryError: String?

    // The driver's Retry with no failed queries runs every query. A mutation
    // error belongs to its button and must not offer that action.
    var retry: (() throws -> Void)? {
        guard live.acceptsInteractions, !live.queriesPaused,
              live.queries.errors.contains(where: { $0.source == "query" }) else { return nil }
        return live.retryFailedQueries
    }

    var body: some View {
        VStack(spacing: 0) {
            PaneTitle(title: "Queries")
            List {
                if case let .object(values) = live.queries.values {
                    ForEach(values, id: \.key) { name, value in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(name).bold()
                            Text(value.compact).textSelection(.enabled)
                        }
                        .font(.callout.monospaced())
                    }
                }
                status("Loading", live.queries.loading)
                status("Refetching", live.queries.refetching)
                if let message = live.queries.evaluationError { Text(message).foregroundStyle(.red) }
                ForEach(Array(live.queries.errors.enumerated()), id: \.offset) { _, issue in
                    VStack(alignment: .leading, spacing: 2) {
                        Text([issue.statementId, issue.code].compactMap { $0 }.joined(separator: " · ")).bold()
                        Text(issue.message)
                        if let hint = issue.hint { Text(hint).foregroundStyle(.secondary) }
                    }
                    .font(.callout).textSelection(.enabled)
                }
                if let retry {
                    Button("Retry failed queries") {
                        do {
                            try retry()
                            retryError = nil
                        }
                        catch { retryError = String(describing: error) }
                    }
                }
                if let retryError { Text(retryError).foregroundStyle(.red) }
            }
        }
    }

    @ViewBuilder private func status(_ label: String, _ names: [String]) -> some View {
        if !names.isEmpty {
            LabeledContent(label, value: names.joined(separator: ", "))
        }
    }
}
