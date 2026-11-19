import Charts
import SwiftUI

/// Draws an element: as the app's own component when the app has one for it,
/// natively when its component is one this package knows and its props make
/// sense, and otherwise as the app's fallback.
///
/// Drawn from a `LiveProgram`, it draws the evaluated tree and redraws when
/// the store changes; buttons run their actions and inputs read and write
/// state. An element drawn on its own (a parse result, or a child an app's
/// component draws) takes the live program from the environment, if any.
/// An element of an evaluated tree knows its own path, so an app's component
/// draws its children with `ElementView(element: child)` and nothing more.
public struct ElementView: View {
    private enum Source {
        case element(Element, path: String)
        case live(LiveProgram)
    }

    private let source: Source
    @Environment(\.elementDrawing) private var drawing

    /// Draws `element` at its own path in the evaluated tree it came from;
    /// `path` is used only for an element without one (a parse result's).
    public init(element: Element, path: String = ElementPath.root) {
        source = .element(element, path: element.path ?? path)
    }

    /// Draws the live program's evaluated tree; nothing while it has no root.
    public init(live: LiveProgram) {
        source = .live(live)
    }

    public var body: some View {
        switch source {
        case let .element(element, path):
            if let drawn = drawing.component(element, path) ?? NativeComponents.view(for: element, path: path) {
                drawing.chrome(drawn, element, path)
            } else {
                drawing.fallback(element, path)
            }
        case let .live(live):
            LiveRoot(live: live).id(ObjectIdentifier(live))
        }
    }
}

/// A live program's tree, and the view lifecycle its queries follow: they
/// pause when the last view drawing the program disappears and resume when
/// one appears.
private struct LiveRoot: View {
    let live: LiveProgram

    var body: some View {
        ZStack {
            if let root = live.root {
                ElementView(element: root).environment(\.liveProgram, live)
            }
        }
        .onAppear { live.viewAppeared() }
        .onDisappear { live.viewDisappeared() }
    }
}

/// How an app extends the drawing, set with `.environment(\.elementDrawing, …)`.
public struct ElementDrawing: Sendable {
    public typealias Draw = @MainActor @Sendable (Element, _ path: String) -> AnyView

    /// The app's own components, asked first; nil leaves the element to this package.
    public var component: @MainActor @Sendable (Element, _ path: String) -> AnyView?
    /// Wraps every component drawn, the app's or this package's.
    public var chrome: @MainActor @Sendable (AnyView, Element, _ path: String) -> AnyView
    /// Draws an element nothing else could: an unknown component, or props that make no sense.
    public var fallback: Draw

    /// With nothing given: no components of the app's own, no wrapping, and
    /// the component's name where an element couldn't be drawn.
    public init(
        component: @escaping @MainActor @Sendable (Element, _ path: String) -> AnyView? = { _, _ in nil },
        chrome: (@MainActor @Sendable (AnyView, Element, _ path: String) -> AnyView)? = nil,
        fallback: Draw? = nil
    ) {
        self.component = component
        self.chrome = chrome ?? { view, _, path in AnyView(view.id(path)) }
        self.fallback = fallback ?? { element, path in AnyView(UnknownElement(element: element).id(path)) }
    }
}

extension EnvironmentValues {
    @Entry public var elementDrawing = ElementDrawing()
    /// The live program being drawn, for components that read or write its
    /// state or run its actions; nil when drawing a parse result.
    @Entry public var liveProgram: LiveProgram? = nil
    /// The name of the form being drawn, for inputs' fields and actions' events.
    @Entry public var formName: String? = nil
}

/// The default fallback: the component's name, marked as not drawn.
struct UnknownElement: View {
    let element: Element

    var body: some View {
        Label(element.typeName, systemImage: "questionmark.square.dashed")
            .font(.callout.monospaced())
            .foregroundStyle(.secondary)
    }
}

/// The components drawn natively: the most used in upstream's example library.
/// Their child parts (columns, select items, series, slices, tab items) are
/// drawn by their parents.
@MainActor
public enum NativeComponents {
    public static let names: Set<String> = [
        "TextContent", "Stack", "Card", "CardHeader", "Button", "Buttons", "Table", "Form",
        "FormControl", "Input", "Select", "LineChart", "BarChart", "PieChart", "Tabs", "Tag",
        "Callout", "TextCallout", "TextArea", "RadioGroup", "CheckBoxGroup", "Slider", "DatePicker", "Chips", "OptionCards",
    ]

    /// The native view for `element`, or nil to draw it as a labeled box.
    public static func view(for element: Element, path: String) -> AnyView? {
        let props = PropReader(element: element)
        do {
            switch element.typeName {
            case "TextContent": return AnyView(try TextContentView(props))
            case "Stack": return AnyView(try StackView(props, path: path, card: nil))
            case "Card": return AnyView(try StackView(props, path: path, card: props.keyword("variant", "card")))
            case "CardHeader": return AnyView(try CardHeaderView(props))
            case "Button": return AnyView(try ButtonView(props))
            case "Buttons": return AnyView(try ButtonsView(props, path: path))
            case "Table": return AnyView(try TableView(props, path: path))
            case "Form": return AnyView(try FormView(props, path: path))
            case "FormControl": return AnyView(try FormControlView(props, path: path))
            case "Input": return AnyView(try InputView(props))
            case "Select": return AnyView(try SelectView(props))
            case "TextArea", "RadioGroup", "CheckBoxGroup", "Slider", "DatePicker", "Chips", "OptionCards":
                return AnyView(try SystemFieldView(props))
            case "LineChart": return AnyView(try SeriesChart(props, kind: .line))
            case "BarChart": return AnyView(try SeriesChart(props, kind: .bar))
            case "PieChart": return AnyView(try PieChartView(props))
            case "Tabs": return AnyView(try TabsView(props, path: path))
            case "Tag": return AnyView(try TagView(props))
            case "Callout", "TextCallout": return AnyView(try CalloutView(props))
            default: return nil
            }
        } catch {
            return nil
        }
    }
}

// MARK: Pieces shared by the components

/// A list prop's items: elements drawn as elements, text as text, nulls skipped.
struct Items: View {
    let items: [PropValue]
    /// The list's path; item `i` sits at `<path>/i`.
    let path: String
    let stretch: Bool

    var body: some View {
        ForEach(Array(items.enumerated()), id: \.offset) { i, item in
            Item(value: item, path: ElementPath.child(of: path, String(i)))
                .frame(maxWidth: stretch ? .infinity : nil, alignment: .leading)
        }
    }
}

struct Item: View {
    let value: PropValue
    let path: String

    var body: some View {
        switch value {
        case let .element(element): ElementView(element: element, path: path)
        case .scalar(.null): EmptyView()
        case let .scalar(.string(s)): Text(s)
        case .expression: PieceText(piece: .dynamic(value.summary))
        default: Text(value.summary).font(.body.monospaced()).foregroundStyle(.secondary)
        }
    }
}

struct PieceText: View {
    let piece: TextPiece

    var body: some View {
        switch piece {
        case let .literal(s):
            Text(s)
        case let .dynamic(source):
            Text(source)
                .italic()
                .foregroundStyle(.secondary)
                .help("Not evaluated: its expression failed or was cut (see the runtime errors), or there's no runtime")
        }
    }
}

extension TextPiece {
    var plain: String {
        switch self {
        case let .literal(s): s
        case let .dynamic(source): source
        }
    }
}

enum Spacing {
    static func of(_ gap: String) -> CGFloat {
        switch gap {
        case "none": 0
        case "xs": 4
        case "s": 8
        case "l": 16
        case "xl": 24
        case "2xl": 32
        default: 12
        }
    }
}

enum Tone {
    static func color(_ variant: String) -> Color {
        switch variant {
        case "info": .blue
        case "success": .green
        case "warning": .orange
        case "error", "danger": .red
        default: .gray
        }
    }

    static func symbol(_ variant: String) -> String {
        switch variant {
        case "info": "info.circle.fill"
        case "success": "checkmark.circle.fill"
        case "warning": "exclamationmark.triangle.fill"
        case "error", "danger": "xmark.octagon.fill"
        default: "text.bubble.fill"
        }
    }
}

// MARK: Text and layout

struct TextContentView: View {
    let text: TextPiece
    let size: String

    init(_ props: PropReader) throws {
        text = try props.text("text")
        size = props.keyword("size", "default")
    }

    var body: some View {
        PieceText(piece: text)
            .font(font)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var font: Font {
        switch size {
        case "small": .callout
        case "large": .title3
        case "small-heavy": .callout.weight(.semibold)
        case "large-heavy": .title.weight(.bold)
        default: .body
        }
    }
}

/// Stack, and Card, which is a stack on a surface.
struct StackView: View {
    let children: [PropValue]
    let path: String
    let row: Bool
    let gap: CGFloat
    let align: String
    let justify: String
    let wrap: Bool
    /// The card variant, or nil for a plain stack.
    let card: String?

    init(_ props: PropReader, path: String, card: String?) throws {
        children = try props.list("children")
        self.path = ElementPath.child(of: path, "children")
        row = props.keyword("direction", "column") == "row"
        gap = Spacing.of(props.keyword("gap", "m"))
        align = props.keyword("align", row ? "start" : "stretch")
        justify = props.keyword("justify", "start")
        wrap = props.bool("wrap")
        self.card = card
    }

    var body: some View {
        surface(stack)
    }

    @ViewBuilder private var stack: some View {
        if row && wrap {
            FlowLayout(spacing: gap) {
                Items(items: children, path: path, stretch: false)
            }
        } else if row {
            HStack(alignment: vertical, spacing: gap) {
                if ["center", "end", "around", "evenly"].contains(justify) { Spacer(minLength: 0) }
                ForEach(Array(children.enumerated()), id: \.offset) { i, child in
                    if i > 0, ["between", "around", "evenly"].contains(justify) { Spacer(minLength: 0) }
                    Item(value: child, path: ElementPath.child(of: path, String(i)))
                        .frame(maxWidth: justify == "start" ? .infinity : nil, alignment: .topLeading)
                }
                if ["center", "around", "evenly"].contains(justify) { Spacer(minLength: 0) }
            }
        } else {
            // Lazy, so only what's on screen is laid out: on the largest corpus
            // program, eager stacks made the worst redraw 267 ms, lazy ones 48
            // (ticket 05).
            LazyVStack(alignment: horizontal, spacing: gap) {
                Items(items: children, path: path, stretch: align == "stretch")
            }
            .frame(maxWidth: .infinity, alignment: Alignment(horizontal: horizontal, vertical: .top))
        }
    }

    private var horizontal: HorizontalAlignment {
        switch align {
        case "center": .center
        case "end": .trailing
        default: .leading
        }
    }

    private var vertical: VerticalAlignment {
        switch align {
        case "center": .center
        case "end": .bottom
        case "baseline": .firstTextBaseline
        default: .top
        }
    }

    @ViewBuilder private func surface(_ content: some View) -> some View {
        switch card {
        case nil:
            content
        case "sunk":
            content.padding(16).background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: 12))
        case "clear":
            content.padding(4)
        default:
            content
                .padding(16)
                .background(.background, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(.separator))
                .shadow(color: .black.opacity(0.06), radius: 2, y: 1)
        }
    }
}

/// Lays children out in rows, wrapping when a row is full.
struct FlowLayout: Layout {
    let spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        let width = rows.map(\.width).max() ?? 0
        return CGSize(width: proposal.width ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for i in row.indices {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row {
        var indices: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = [Row()]
        for i in subviews.indices {
            let size = subviews[i].sizeThatFits(.unspecified)
            let needed = rows[rows.count - 1].indices.isEmpty ? size.width : rows[rows.count - 1].width + spacing + size.width
            if needed > width, !rows[rows.count - 1].indices.isEmpty {
                rows.append(Row())
            }
            let last = rows.count - 1
            rows[last].width += (rows[last].indices.isEmpty ? 0 : spacing) + size.width
            rows[last].height = max(rows[last].height, size.height)
            rows[last].indices.append(i)
        }
        return rows
    }
}

struct CardHeaderView: View {
    let title: TextPiece?
    let subtitle: TextPiece?

    init(_ props: PropReader) throws {
        title = try props.optionalText("title")
        subtitle = try props.optionalText("subtitle")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            if let title { PieceText(piece: title).font(.headline) }
            if let subtitle { PieceText(piece: subtitle).font(.subheadline).foregroundStyle(.secondary) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

// MARK: Buttons and tags

struct ButtonView: View {
    let label: TextPiece
    let variant: String
    let destructive: Bool
    let size: String
    /// Upstream's primary button, which checks its form's fields first: the
    /// variant is "primary", or falsy and so primary by default.
    let primary: Bool
    /// The element it was drawn from: a click runs this element's action, if
    /// it's still what the tree holds.
    let element: Element
    @Environment(\.liveProgram) private var live
    @Environment(\.formName) private var formName
    @Environment(\.formContext) private var form

    init(_ props: PropReader) throws {
        label = try props.text("label")
        variant = props.keyword("variant", "primary")
        destructive = props.keyword("type", "normal") == "destructive"
        size = props.keyword("size", "medium")
        primary = switch props.value("variant") {
        case nil, .scalar(.string("primary"))?, .scalar(.string(""))?, .scalar(.bool(false))?, .scalar(.number(0))?: true
        default: false
        }
        element = props.element
    }

    var body: some View {
        let button = Button(role: destructive ? .destructive : nil) {
            guard let live, live.acceptsInteractions else { return }
            live.attempt {
                guard let element = live.current(element, form: formName) else { throw LiveProgram.stale }
                if let form, primary, Self.validates(PropReader(element: element).value("action")),
                   !form.validation.validate(enclosing: element, in: live, formName: formName) {
                    return
                }
                try live.run("action", of: element, message: label.plain, form: formName)
            }
        } label: {
            PieceText(piece: label)
        }
        .controlSize(controlSize)
        .disabled(!(live?.acceptsInteractions ?? true))
        switch variant {
        case "secondary": button.buttonStyle(.bordered)
        case "tertiary": button.buttonStyle(.borderless)
        default: button.buttonStyle(.borderedProminent).tint(destructive ? .red : .accentColor)
        }
    }

    /// Whether a primary button checks its form before running `action`, as
    /// upstream's does: for a plan, when a step sends something away (a
    /// message to the assistant, a custom action, a mutation); for an older
    /// action object or no action at all, always.
    static func validates(_ action: PropValue?) -> Bool {
        guard case let .plan(plan)? = action else { return true }
        return plan.steps.contains { step in
            switch step["type"]?.string {
            case "continue_conversation", "custom_action": true
            case "run": step["refType"] == .string("mutation")
            default: false
            }
        }
    }

    private var controlSize: ControlSize {
        switch size {
        case "extra-small": .mini
        case "small": .small
        case "large": .large
        default: .regular
        }
    }
}

struct ButtonsView: View {
    let buttons: [PropValue]
    let path: String
    let row: Bool

    init(_ props: PropReader, path: String) throws {
        buttons = try props.list("buttons")
        self.path = ElementPath.child(of: path, "buttons")
        row = props.keyword("direction", "row") == "row"
    }

    var body: some View {
        if row {
            HStack(spacing: 8) { Items(items: buttons, path: path, stretch: false) }
                .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            VStack(alignment: .leading, spacing: 8) { Items(items: buttons, path: path, stretch: false) }
        }
    }
}

struct TagView: View {
    let text: TextPiece
    let size: String
    let variant: String

    init(_ props: PropReader) throws {
        text = try props.text("text")
        size = props.keyword("size", "md")
        variant = props.keyword("variant", "neutral")
    }

    var body: some View {
        PieceText(piece: text)
            .font(size == "sm" ? .caption : size == "lg" ? .body : .callout)
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .foregroundStyle(Tone.color(variant))
            .background(Tone.color(variant).opacity(0.15), in: Capsule())
    }
}

struct CalloutView: View {
    let variant: String
    let title: TextPiece?
    let description: TextPiece?

    init(_ props: PropReader) throws {
        variant = props.keyword("variant", "neutral")
        title = try props.optionalText("title")
        description = try props.optionalText("description")
        if title == nil, description == nil, !props.element.partial { throw PropReader.Unusable() }
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: Tone.symbol(variant)).foregroundStyle(Tone.color(variant))
            VStack(alignment: .leading, spacing: 2) {
                if let title { PieceText(piece: title).bold() }
                if let description { PieceText(piece: description).foregroundStyle(.secondary) }
            }
            Spacer(minLength: 0)
        }
        .padding(12)
        .background(Tone.color(variant).opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Tone.color(variant).opacity(0.3)))
    }
}

// MARK: Tables and forms

struct TableView: View {
    struct Column {
        var label: String
        var type: String
    }

    let columns: [Column]
    /// Rows of cells; a row that isn't a list is drawn as one line of text.
    let rows: [PropValue]
    /// Where the rows sit: cell `c` of row `r` is at `<rows>/r/c`.
    let rowsPath: String
    /// The table it was drawn from: an action cell runs only while this is
    /// still what the tree holds.
    let element: Element
    @Environment(\.liveProgram) private var live
    @Environment(\.formName) private var formName
    @Environment(\.formContext) private var form

    init(_ props: PropReader, path: String) throws {
        columns = try props.elements("columns", of: "Col").map { _, col in
            let reader = PropReader(element: col)
            return Column(label: try reader.text("label").plain, type: reader.keyword("type", "string"))
        }
        rows = try props.list("rows")
        rowsPath = ElementPath.child(of: path, "rows")
        element = props.element
    }

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 8) {
            GridRow {
                ForEach(Array(columns.enumerated()), id: \.offset) { _, column in
                    Text(column.label).font(.callout.weight(.semibold)).foregroundStyle(.secondary)
                        .gridColumnAlignment(column.type == "number" ? .trailing : .leading)
                }
            }
            Divider()
            ForEach(Array(rows.enumerated()), id: \.offset) { r, row in
                GridRow {
                    if case let .list(cells) = row {
                        ForEach(Array(columns.indices), id: \.self) { c in
                            cell(c < cells.count ? cells[c] : nil, type: columns[c].type, row: r, column: c)
                        }
                    } else {
                        Text(row.summary).foregroundStyle(.secondary).gridCellColumns(max(columns.count, 1))
                    }
                }
            }
        }
    }

    /// An action cell is a button that, like one with no action, asks to
    /// continue the conversation with its text; an element is drawn.
    @ViewBuilder private func cell(_ value: PropValue?, type: String, row: Int, column: Int) -> some View {
        switch value {
        case nil, .scalar(.null)?:
            Text("")
        case let .scalar(.string(s))? where type == "action":
            Button(s) {
                guard let live, live.acceptsInteractions else { return }
                live.attempt {
                    guard let current = live.current(element, form: formName) else { throw LiveProgram.stale }
                    if let form, !form.validation.validate(enclosing: current, in: live, formName: formName) { return }
                    try live.runActionCell(row: row, column: column, of: current, form: formName)
                }
            }
            .controlSize(.small)
            .disabled(!(live?.acceptsInteractions ?? true))
        case let .element(element)?:
            ElementView(element: element, path: ElementPath.child(of: ElementPath.child(of: rowsPath, String(row)), String(column)))
        case let .scalar(.number(n))?:
            Text(JSONValue.format(n)).monospacedDigit()
        case let .scalar(.string(s))?:
            Text(s)
        case let value?:
            Text(value.summary).foregroundStyle(.secondary)
        }
    }
}

struct FormView: View {
    let name: String
    let fields: [PropValue]
    let buttons: Element?
    let path: String
    /// The fields' errors from the last check, kept while the form is drawn.
    @State private var validation = FormValidation()

    init(_ props: PropReader, path: String) throws {
        name = try props.text("name").plain
        fields = try props.list("fields")
        buttons = try props.optionalElement("buttons")
        self.path = path
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Items(items: fields, path: ElementPath.child(of: path, "fields"), stretch: true)
            if let buttons {
                ElementView(element: buttons, path: ElementPath.child(of: path, "buttons"))
            }
        }
        .environment(\.formName, name)
        .environment(\.formContext, FormContext(validation: validation))
    }
}

/// A labelled field. Inside a form, as upstream's does, it shows its input's
/// error in place of the hint and marks a field whose rules say `required`.
struct FormControlView: View {
    let label: TextPiece
    let input: Element?
    let hint: TextPiece?
    let path: String
    /// The input's name, which its error is kept under.
    let fieldName: String?
    let required: Bool
    @Environment(\.formContext) private var form

    init(_ props: PropReader, path: String) throws {
        label = try props.text("label")
        input = try props.element("input")
        hint = try props.optionalText("hint")
        self.path = path
        let inputProps = input.map { PropReader(element: $0) }
        fieldName = switch inputProps?.value("name") {
        case let .scalar(.string(name))?: name
        case let .record(fields)?: fields.first { $0.name == "name" }?.value.json.string
        default: nil
        }
        if case let .record(rules)? = inputProps?.value("rules"),
           case .scalar(.bool(true))? = rules.first(where: { $0.name == "required" })?.value {
            required = true
        } else {
            required = false
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 2) {
                PieceText(piece: label)
                if required { Text("*").foregroundStyle(.red) }
            }
            .font(.callout.weight(.medium))
            if let input {
                ElementView(element: input, path: ElementPath.child(of: path, "input"))
            }
            if let error = fieldName.flatMap({ form?.validation.errors[$0] }) {
                Label(error, systemImage: "exclamationmark.circle")
                    .font(.caption)
                    .foregroundStyle(.red)
            } else if let hint {
                PieceText(piece: hint).font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

/// What an input or select shows and where typing goes: a binding marker's
/// state variable, or else the form field named by the input's name, which
/// starts as the value prop.
@MainActor struct FieldValue {
    let name: String
    let value: PropValue?
    /// The input it was drawn from; it writes only while that's still the
    /// one in the tree.
    let element: Element

    init(_ props: PropReader, name: String) {
        self.name = name
        value = props.value("value")
        element = props.element
    }

    /// What the field holds: the bound state variable, or the form field
    /// (until it has a value, the value prop).
    func value(in live: LiveProgram?, form: String?, includingCompositeDefaults: Bool = false) -> JSONValue? {
        switch value {
        case let .binding(marker)?: live?.value(of: marker.target)
        case let .scalar(json)?: live?.field(form: form, name: name) ?? json
        case let value? where includingCompositeDefaults: live?.field(form: form, name: name) ?? value.json
        default: live?.field(form: form, name: name)
        }
    }

    func text(in live: LiveProgram?, form: String?) -> String {
        switch value(in: live, form: form) {
        case nil, .null?: return ""
        case let .string(s)?: return s
        case let .number(n)?: return JSONValue.format(n)
        case let other?: return other.compact
        }
    }

    /// Writes what was typed or picked. SwiftUI hands a field's own new value
    /// back when it redraws one that isn't being edited; that changes nothing,
    /// so it isn't written again (through a transform, it would compound).
    /// Returns whether it was written.
    @discardableResult
    func write(_ text: String, to live: LiveProgram?, form: String?) -> Bool {
        guard let live, text != self.text(in: live, form: form) else { return false }
        live.attempt { try live.enter(.string(text), into: element, form: form) }
        return live.failure == nil
    }
}

/// A text field. In a form, as upstream's: focusing it clears its error,
/// typing clears it when it has rules, and leaving it checks them.
struct InputView: View {
    let placeholder: String
    let secure: Bool
    let field: FieldValue
    let rules: [ValidationRule]
    @Environment(\.liveProgram) private var live
    @Environment(\.formName) private var formName
    @Environment(\.formContext) private var form
    @FocusState private var focused: Bool

    init(_ props: PropReader) throws {
        let name = try props.text("name").plain
        placeholder = try props.optionalText("placeholder")?.plain ?? name
        secure = props.keyword("type", "text") == "password"
        field = FieldValue(props, name: name)
        rules = Validation.rules(props.value("rules"))
    }

    var body: some View {
        let text = Binding(
            get: { field.text(in: live, form: formName) },
            set: { typed in
                if field.write(typed, to: live, form: formName), !rules.isEmpty { form?.validation.clear(field.name) }
            }
        )
        Group {
            if secure {
                SecureField(placeholder, text: text)
            } else {
                TextField(placeholder, text: text)
            }
        }
        .focused($focused)
        .disabled(!(live?.acceptsInteractions ?? true))
        .onChange(of: focused) {
            guard let form else { return }
            if focused {
                form.validation.clear(field.name)
            } else if !rules.isEmpty {
                form.validation.validate(field.name, value: field.value(in: live, form: formName), rules: rules)
            }
        }
    }
}

/// A pop-up menu. In a form, picking checks its rules, as upstream's does.
struct SelectView: View {
    let placeholder: String
    let items: [(value: String, label: String)]
    let field: FieldValue
    let rules: [ValidationRule]
    @Environment(\.liveProgram) private var live
    @Environment(\.formName) private var formName
    @Environment(\.formContext) private var form

    init(_ props: PropReader) throws {
        let name = try props.text("name").plain
        placeholder = try props.optionalText("placeholder")?.plain ?? name
        items = try props.elements("items", of: "SelectItem").map { _, item in
            let reader = PropReader(element: item)
            return (try reader.text("value").plain, try reader.text("label").plain)
        }
        field = FieldValue(props, name: name)
        rules = Validation.rules(props.value("rules"))
    }

    var body: some View {
        let selection = Binding(
            get: { field.text(in: live, form: formName) },
            set: { picked in
                if field.write(picked, to: live, form: formName), !rules.isEmpty {
                    form?.validation.validate(field.name, value: field.value(in: live, form: formName), rules: rules)
                }
            }
        )
        Picker(placeholder, selection: selection) {
            Text(placeholder).tag("")
            ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                Text(item.label).tag(item.value)
            }
        }
        .labelsHidden()
        .fixedSize()
        .disabled(!(live?.acceptsInteractions ?? true))
    }
}

// MARK: Charts

struct ChartPoint: Identifiable {
    var id: Int
    var series: String
    var label: String
    var value: Double
}

struct SeriesChart: View {
    enum Kind { case line, bar }

    let kind: Kind
    let points: [ChartPoint]
    let variant: String
    let xLabel: String
    let yLabel: String

    init(_ props: PropReader, kind: Kind) throws {
        self.kind = kind
        let labels = try props.strings("labels")
        var points: [ChartPoint] = []
        for (_, series) in try props.elements("series", of: "Series") {
            let reader = PropReader(element: series)
            let category = try reader.text("category").plain
            for (i, value) in try reader.numbers("values").enumerated() where i < labels.count {
                points.append(ChartPoint(id: points.count, series: category, label: labels[i], value: value))
            }
        }
        self.points = points
        variant = props.keyword("variant", kind == .line ? "linear" : "stacked")
        xLabel = props.keyword("xLabel", "")
        yLabel = props.keyword("yLabel", "")
    }

    var body: some View {
        Chart(points) { point in
            switch kind {
            case .line:
                LineMark(x: .value("x", point.label), y: .value("y", point.value))
                    .foregroundStyle(by: .value("Series", point.series))
                    .interpolationMethod(variant == "natural" ? .catmullRom : variant == "step" ? .stepCenter : .linear)
            case .bar:
                if variant == "grouped" {
                    BarMark(x: .value("x", point.label), y: .value("y", point.value))
                        .foregroundStyle(by: .value("Series", point.series))
                        .position(by: .value("Series", point.series))
                } else {
                    BarMark(x: .value("x", point.label), y: .value("y", point.value))
                        .foregroundStyle(by: .value("Series", point.series))
                }
            }
        }
        .modifier(AxisTitles(x: xLabel, y: yLabel))
        .frame(minHeight: 180)
    }
}

/// Axis titles, only where the program gives them.
struct AxisTitles: ViewModifier {
    let x: String
    let y: String

    func body(content: Content) -> some View {
        switch (x.isEmpty, y.isEmpty) {
        case (true, true): content
        case (false, true): content.chartXAxisLabel(x)
        case (true, false): content.chartYAxisLabel(y)
        case (false, false): content.chartXAxisLabel(x).chartYAxisLabel(y)
        }
    }
}

struct PieChartView: View {
    let slices: [ChartPoint]
    let donut: Bool

    init(_ props: PropReader) throws {
        slices = try props.elements("slices", of: "Slice").enumerated().map { i, entry in
            let reader = PropReader(element: entry.element)
            return ChartPoint(id: i, series: try reader.text("category").plain, label: "", value: try reader.number("value"))
        }
        donut = props.keyword("variant", "pie") == "donut"
    }

    var body: some View {
        Chart(slices) { slice in
            SectorMark(angle: .value("Value", slice.value), innerRadius: .ratio(donut ? 0.6 : 0), angularInset: 1)
                .foregroundStyle(by: .value("Category", slice.series))
        }
        .frame(minHeight: 180)
    }
}

// MARK: Tabs

struct TabsView: View {
    struct Tab {
        var trigger: TextPiece
        var content: [PropValue]
        /// The tab item's index, for its content's paths.
        var index: Int
    }

    let tabs: [Tab]
    let path: String
    @State private var selected = 0
    @State private var previewSelected = 0
    @Environment(\.liveProgram) private var live
    private var isPreview: Bool { live?.showingPreview ?? false }
    private var displayedSelection: Int { isPreview ? previewSelected : selected }
    @Environment(\.formContext) private var form

    init(_ props: PropReader, path: String) throws {
        tabs = try props.elements("items", of: "TabItem").map { index, item in
            let reader = PropReader(element: item)
            return Tab(trigger: try reader.text("trigger"), content: try reader.list("content"), index: index)
        }
        self.path = ElementPath.child(of: path, "items")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Picker("", selection: Binding(get: { displayedSelection }, set: {
                if isPreview { previewSelected = $0 }
                else {
                    selected = $0
                    form?.validation.selectedTabs[path] = $0
                }
            })) {
                ForEach(Array(tabs.enumerated()), id: \.offset) { i, tab in
                    Text(tab.trigger.plain).tag(i)
                }
            }
            .onAppear { form?.validation.selectedTabs[path] = selected }
            .onDisappear { form?.validation.selectedTabs[path] = nil }
            .onChange(of: isPreview) {
                if !isPreview { previewSelected = 0 }
                form?.validation.selectedTabs[path] = selected
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            if tabs.indices.contains(displayedSelection) {
                let tab = tabs[displayedSelection]
                VStack(alignment: .leading, spacing: 12) {
                    Items(
                        items: tab.content,
                        path: ElementPath.child(of: ElementPath.child(of: path, String(tab.index)), "content"),
                        stretch: true
                    )
                }
            }
        }
    }
}
