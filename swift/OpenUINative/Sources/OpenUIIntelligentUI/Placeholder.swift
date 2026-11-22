// Until a component is drawn natively it shows a placeholder that names it,
// with the components inside its props drawn beneath, so a whole program
// shows and partial progress is visible.
import OpenUINative
import SwiftUI

/// Declares one of the library's components, drawn by the placeholder.
func component(_ name: String, _ description: String, _ props: [PropDefinition]) -> DefinedComponent {
    defineComponent(name: name, description: description, props: props) { element, _ in
        Placeholder(element: element)
    }
}

struct Placeholder: View {
    let element: Element

    /// What the placeholder says: the component's name.
    var title: String { element.typeName }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(title, systemImage: "square.dashed")
                .font(.callout.monospaced())
                .foregroundStyle(.secondary)
            ForEach(Array(Placeholder.children(of: element).enumerated()), id: \.offset) { _, child in
                ElementView(element: child)
            }
        }
        .padding(8)
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.secondary, style: StrokeStyle(lineWidth: 1, dash: [4])))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(title)
    }

    /// The elements in an element's props, in order, not looking inside the
    /// elements themselves: each draws its own. Walks an explicit stack, so
    /// no depth of nesting adds a call frame.
    static func children(of element: Element) -> [Element] {
        var found: [Element] = []
        var pending: [PropValue] = element.props.reversed().map(\.value)
        while let value = pending.popLast() {
            switch value {
            case let .element(child): found.append(child)
            case let .list(items): pending.append(contentsOf: items.reversed())
            case let .record(fields): pending.append(contentsOf: fields.reversed().map(\.value))
            case .expression, .binding, .plan, .scalar: continue
            }
        }
        return found
    }
}
