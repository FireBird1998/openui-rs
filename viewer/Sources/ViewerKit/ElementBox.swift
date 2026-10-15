import SwiftUI

/// Draws any element as a labeled box: its type, its props, and the elements
/// inside it drawn the same way. Every program draws something this way.
/// Partial elements get a dashed orange border; clicking a box selects it.
public struct ElementBox: View {
    let element: Element
    let path: String
    @Environment(ElementSelection.self) private var selection: ElementSelection?

    public init(element: Element, path: String = ElementPath.root) {
        self.element = element
        self.path = path
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text(element.typeName).font(.headline.monospaced())
                if let id = element.statementId {
                    Text(id).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                if element.partial {
                    Image(systemName: "ellipsis")
                        .font(.caption)
                        .foregroundStyle(.orange)
                        .help("Partial: still arriving, or waiting on something that hasn't arrived")
                }
            }
            ForEach(Array(element.props.enumerated()), id: \.offset) { _, prop in
                PropRow(prop: prop, path: ElementPath.child(of: path, prop.name))
            }
        }
        .padding(8)
        .frame(alignment: .leading)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 6))
        .overlay(
            RoundedRectangle(cornerRadius: 6).strokeBorder(
                element.partial ? AnyShapeStyle(.orange.opacity(0.7)) : AnyShapeStyle(.secondary.opacity(0.5)),
                style: StrokeStyle(lineWidth: 1, dash: element.partial ? [4, 3] : [])
            )
        )
        .overlay(SelectionRing(path: path))
        .contentShape(Rectangle())
        .onTapGesture { selection?.select(path, from: .drawing) }
        .id(path)
    }
}

/// The selected box's outline; only these small views redraw when the selection changes.
struct SelectionRing: View {
    let path: String
    @Environment(ElementSelection.self) private var selection: ElementSelection?

    var body: some View {
        if selection?.path == path {
            RoundedRectangle(cornerRadius: 6).strokeBorder(Color.accentColor, lineWidth: 2)
        }
    }
}

/// One prop: on one line when it holds no elements, otherwise its elements
/// drawn below its name.
struct PropRow: View {
    let prop: Prop
    /// The prop's path: where the elements in it sit.
    let path: String

    var body: some View {
        if prop.value.holdsElements {
            VStack(alignment: .leading, spacing: 4) {
                PropName(name: prop.name)
                PropValueView(value: prop.value, path: path).padding(.leading, 12)
            }
        } else {
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                PropName(name: prop.name)
                Text(prop.value.summary).font(.body.monospaced())
            }
        }
    }
}

struct PropName: View {
    let name: String

    var body: some View {
        Text(name + ":").font(.callout.monospaced()).foregroundStyle(.secondary)
    }
}

/// A value that holds elements somewhere inside it.
struct PropValueView: View {
    let value: PropValue
    let path: String

    var body: some View {
        switch value {
        case let .element(element):
            ElementBox(element: element, path: path)
        case let .list(items):
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(items.enumerated()), id: \.offset) { i, item in
                    if item.holdsElements {
                        PropValueView(value: item, path: ElementPath.child(of: path, String(i)))
                    } else {
                        Text(item.summary).font(.body.monospaced())
                    }
                }
            }
        case let .record(fields):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(fields.enumerated()), id: \.offset) { _, field in
                    PropRow(prop: field, path: ElementPath.child(of: path, field.name))
                }
            }
        case .expression, .scalar:
            Text(value.summary).font(.body.monospaced())
        }
    }
}

extension PropValue {
    var holdsElements: Bool {
        switch self {
        case .element: true
        case let .list(items): items.contains { $0.holdsElements }
        case let .record(fields): fields.contains { $0.value.holdsElements }
        case .expression, .scalar: false
        }
    }

    /// One line of text for a value with no elements in it.
    var summary: String {
        switch self {
        case let .scalar(json): json.compact
        case let .expression(json): "ƒ " + json.compact
        case let .list(items): "[" + items.map(\.summary).joined(separator: ", ") + "]"
        case let .record(fields): "{" + fields.map { $0.name + ": " + $0.value.summary }.joined(separator: ", ") + "}"
        case let .element(element): element.typeName + "(…)"
        }
    }
}
