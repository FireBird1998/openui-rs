import OpenUINative
import SwiftUI

/// Beside the drawing: the store of the program on screen, each value
/// editable as JSON, then the runtime errors and the host events it has sent.
/// Editing a value sets it in the store, and the drawing redraws.
public struct StorePanel: View {
    let playback: Playback

    public init(playback: Playback) { self.playback = playback }

    public var body: some View {
        if let live = playback.live {
            let rows = StoreRow.rows(of: live)
            List {
                if let failure = live.failure {
                    Text(failure).foregroundStyle(.red).font(.callout).textSelection(.enabled)
                }
                Section("Store (\(rows.count))") {
                    ForEach(rows) { row in
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
                Section(playback.droppedHostEvents > 0
                        ? "Host events (newest \(playback.hostEvents.count); \(playback.droppedHostEvents) older let go)"
                        : "Host events (\(playback.hostEvents.count))") {
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

    /// Every entry of the store's snapshot, with which are pristine: read
    /// from what the live program already has, with no call per row.
    @MainActor static func rows(of live: LiveProgram) -> [StoreRow] {
        guard case let .object(entries) = live.state else { return [] }
        let pristine = live.pristineNames
        return entries.map { StoreRow(name: $0.key, json: $0.value.compact, pristine: pristine.contains($0.key)) }
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

/// What a store row's text field holds: the value, or the person's edit of
/// it. A value that changes while they're editing doesn't overwrite the
/// edit; it's held as `newer` for them to take or type over.
struct StoreDraft: Equatable {
    /// The field's text.
    var text: String
    /// The value the text started from.
    private(set) var base: String
    /// The value now, when it changed during an edit.
    private(set) var newer: String?

    init(_ value: String) {
        text = value
        base = value
    }

    var isEdited: Bool { text != base }

    /// The value in the store changed.
    mutating func valueChanged(to value: String, editing: Bool) {
        if editing && isEdited {
            newer = value == base ? nil : value
        } else {
            self = StoreDraft(value)
        }
    }

    /// Shows the newer value, dropping the edit.
    mutating func takeNewer() {
        if let newer { self = StoreDraft(newer) }
    }

    /// The edit was set in the store as `value`.
    mutating func committed(as value: String) {
        self = StoreDraft(value)
    }
}

struct StoreRowView: View {
    let row: StoreRow
    let live: LiveProgram
    @State private var draft = StoreDraft("")
    @State private var problem: String?
    @FocusState private var editing: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(row.name).font(.callout.monospaced().bold())
                if row.pristine {
                    Text("starting value").font(.caption).foregroundStyle(.secondary)
                }
            }
            TextField("JSON", text: $draft.text)
                .font(.callout.monospaced())
                .textFieldStyle(.roundedBorder)
                .focused($editing)
                .onSubmit {
                    problem = StoreRow.commit(draft.text, to: row.name, in: live)
                    if problem == nil { draft.committed(as: row.json) }
                }
            if let newer = draft.newer {
                HStack(spacing: 4) {
                    Text("Changed to \(newer) while you were editing")
                        .font(.caption).foregroundStyle(.orange).lineLimit(2)
                    Button("Show it") { draft.takeNewer() }
                        .buttonStyle(.link).font(.caption)
                }
            }
            if let problem {
                Text(problem).font(.caption).foregroundStyle(.red)
            }
        }
        .onAppear { draft = StoreDraft(row.json) }
        .onChange(of: row.json) { draft.valueChanged(to: row.json, editing: editing) }
        .onChange(of: editing) { if !editing && !draft.isEdited { draft.valueChanged(to: row.json, editing: false) } }
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
