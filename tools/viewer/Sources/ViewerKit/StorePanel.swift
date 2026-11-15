import OpenUINative
import SwiftUI

/// Beside the drawing: the store of the program on screen, each value
/// editable as JSON, then the runtime errors and the host events it has sent.
/// Editing a value sets it in the store, and the drawing redraws.
struct StorePanel: View {
    let playback: Playback

    var body: some View {
        if let live = playback.live {
            List {
                if let failure = live.failure {
                    Text(failure).foregroundStyle(.red).font(.callout).textSelection(.enabled)
                }
                Section("Store (\(StoreRow.rows(of: live).count))") {
                    ForEach(StoreRow.rows(of: live)) { row in
                        StoreRowView(row: row, live: live)
                    }
                }
                Section("Runtime errors (\(live.errors.count))") {
                    Flag("Cut at the size limit", on: live.sizeLimited, warn: true)
                    Flag("Cut at the depth limit", on: live.depthLimited, warn: true)
                    ForEach(Array(live.errors.enumerated()), id: \.offset) { _, issue in
                        RuntimeIssueRow(issue: issue)
                    }
                }
                Section("Host events (\(playback.hostEvents.count))") {
                    ForEach(Array(playback.hostEvents.enumerated().reversed()), id: \.offset) { _, event in
                        HostEventRow(event: event)
                    }
                }
            }
        } else {
            ContentUnavailableView("No runtime session", systemImage: "tray")
        }
    }
}

/// One entry in the store, as the panel shows it.
struct StoreRow: Identifiable, Equatable {
    var name: String
    /// The value as compact JSON, which is also how it's edited.
    var json: String
    /// Still the declared starting value.
    var pristine: Bool
    var id: String { name }

    @MainActor static func rows(of live: LiveProgram) -> [StoreRow] {
        guard case let .object(entries) = live.state else { return [] }
        return entries.map { StoreRow(name: $0.key, json: $0.value.compact, pristine: live.isPristine($0.key)) }
    }

    /// Sets `name` to the JSON in `text`; returns why not if it can't be.
    @MainActor static func commit(_ text: String, to name: String, in live: LiveProgram) -> String? {
        do {
            try live.set(name, to: try JSONValue.parse(text))
            return nil
        } catch {
            return String(describing: error)
        }
    }
}

struct StoreRowView: View {
    let row: StoreRow
    let live: LiveProgram
    @State private var draft = ""
    @State private var problem: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(row.name).font(.callout.monospaced().bold())
                if row.pristine {
                    Text("starting value").font(.caption).foregroundStyle(.secondary)
                }
            }
            TextField("JSON", text: $draft)
                .font(.callout.monospaced())
                .textFieldStyle(.roundedBorder)
                .onSubmit { problem = StoreRow.commit(draft, to: row.name, in: live) }
            if let problem {
                Text(problem).font(.caption).foregroundStyle(.red)
            }
        }
        .onAppear { draft = row.json }
        .onChange(of: row.json) { draft = row.json }
        .help("Edit the value as JSON and press Return to set it")
    }
}

struct RuntimeIssueRow: View {
    let issue: RuntimeIssue

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(issue.code).bold().foregroundStyle(.red)
                if let component = issue.component { Text(component) }
                if let statement = issue.statementId {
                    Text("in \(statement)").foregroundStyle(.secondary)
                }
            }
            .font(.callout.monospaced())
            Text(issue.message).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
        }
    }
}

struct HostEventRow: View {
    let event: HostEvent

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(event.type).bold()
                if let form = event.formName {
                    Text("form \(form)").foregroundStyle(.secondary)
                }
            }
            .font(.callout.monospaced())
            if !event.humanFriendlyMessage.isEmpty {
                Text(event.humanFriendlyMessage).font(.caption)
            }
            Text(event.params.compact).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(2)
        }
        .help(event.json.compact)
    }
}
