import SwiftUI

/// The parse result at the shown step: whether it's the reference's, its
/// flags, the element tree, and what's unresolved, orphaned or invalid.
public struct InspectorView: View {
    let playback: Playback
    let golden: GoldenSteps?
    let chunking: Chunking
    @Environment(ElementSelection.self) private var selection

    public init(playback: Playback, golden: GoldenSteps?, chunking: Chunking) {
        self.playback = playback
        self.golden = golden
        self.chunking = chunking
    }

    public var body: some View {
        let result = playback.step?.result
        let rows = result?.root.map(TreeRow.rows(of:)) ?? []
        ScrollViewReader { proxy in
            List(selection: Binding(
                get: { selection.path },
                set: { selection.select($0, from: .tree) }
            )) {
                if let golden {
                    Section("Reference") {
                        GoldenRow(verdict: golden.verdict(step: playback.position, json: playback.step?.json ?? "", chunking: chunking))
                    }
                }
                if let result {
                    Section("This step") {
                        LabeledContent("Step", value: "\(playback.position) of \(playback.chunks.count)")
                        LabeledContent("Statements", value: "\(result.meta.statementCount)")
                        Flag("Incomplete", on: result.meta.incomplete)
                        Flag("Cut at the depth limit", on: result.depthLimited, warn: true)
                        Flag("Cut at the size limit", on: result.sizeLimited, warn: true)
                    }
                    Section("Elements (\(rows.count))") {
                        ForEach(rows) { row in
                            TreeRowLabel(row: row).tag(row.id)
                        }
                    }
                    NameList(title: "Unresolved", names: result.meta.unresolved)
                    NameList(title: "Orphaned", names: result.meta.orphaned)
                    Section("Errors (\(result.meta.errors.count))") {
                        ForEach(Array(result.meta.errors.enumerated()), id: \.offset) { _, error in
                            ErrorRow(error: error)
                        }
                    }
                }
            }
            .onChange(of: selection.revealInTree) {
                if let path = selection.path { proxy.scrollTo(path, anchor: .center) }
            }
        }
    }
}

struct GoldenRow: View {
    let verdict: GoldenVerdict

    var body: some View {
        switch verdict {
        case .matches:
            Label("Matches the reference", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
        case let .differs(reference, viewer):
            VStack(alignment: .leading, spacing: 2) {
                Label("Differs from the reference", systemImage: "xmark.octagon.fill").foregroundStyle(.red)
                Text("reference \(reference), viewer \(viewer)").font(.caption.monospaced()).foregroundStyle(.secondary)
            }
        case .nothingPushed:
            Label("Nothing pushed yet", systemImage: "circle.dashed").foregroundStyle(.secondary)
        case .noGolden:
            Label("No golden for this chunking", systemImage: "questionmark.circle").foregroundStyle(.secondary)
        }
    }
}

struct Flag: View {
    let title: String
    let on: Bool
    let warn: Bool

    init(_ title: String, on: Bool, warn: Bool = false) {
        self.title = title
        self.on = on
        self.warn = warn
    }

    var body: some View {
        LabeledContent(title) {
            Text(on ? "yes" : "no").foregroundStyle(on && warn ? .orange : .primary)
        }
    }
}

struct TreeRowLabel: View {
    let row: TreeRow

    var body: some View {
        HStack(spacing: 4) {
            if let key = row.key {
                Text(key + ":").foregroundStyle(.secondary)
            }
            Text(row.typeName).bold()
            if let id = row.statementId {
                Text(id).foregroundStyle(.secondary)
            }
            if row.partial {
                Image(systemName: "ellipsis").foregroundStyle(.orange)
            }
        }
        .font(.callout.monospaced())
        .lineLimit(1)
        .padding(.leading, CGFloat(row.depth) * 12)
        .id(row.id)
    }
}

struct NameList: View {
    let title: String
    let names: [String]

    var body: some View {
        Section("\(title) (\(names.count))") {
            ForEach(Array(names.enumerated()), id: \.offset) { _, name in
                Text(name).font(.callout.monospaced())
            }
        }
    }
}

struct ErrorRow: View {
    let error: ValidationIssue

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(error.code).bold().foregroundStyle(.red)
                Text(error.component + error.path)
                if let statement = error.statementId {
                    Text("in \(statement)").foregroundStyle(.secondary)
                }
            }
            .font(.callout.monospaced())
            Text(error.message).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
        }
    }
}
