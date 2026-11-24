import AppKit
import OpenUINative
import SwiftUI

// The trigger itself is a button. Rewrite its evaluated values before any
// nested host or library renderer gets them. Walk the admitted tree once.
private enum TriggerTask {
    case visit(PropValue)
    case list(Int)
    case element(Element)

    static func plain(_ trigger: Prop) -> PropValue {
        var tasks = [Self.visit(trigger.value)]
        var values: [PropValue] = []
        while let task = tasks.popLast() {
            switch task {
            case let .visit(.list(list)):
                tasks.append(.list(list.count))
                tasks.append(contentsOf: list.reversed().map(Self.visit))
            case let .visit(.element(element)):
                switch element.typeName {
                case "Pressable":
                    tasks.append(.visit(element.prop("children") ?? .scalar(.null)))
                case "Link":
                    values.append(element.prop("text") ?? .scalar(.null))
                case "IconButton":
                    var icon = element
                    icon.typeName = "Icon"
                    var size = trigger
                    size.name = "size"
                    size.value = .scalar(.string("sm"))
                    icon.props = element.props.first(where: { $0.name == "icon" }).map { prop in
                        var name = prop
                        name.name = "name"
                        return [name, size]
                    } ?? [size]
                    values.append(.element(icon))
                case "Button", "Checkbox", "Input", "TextArea", "Select", "Slider", "RadioGroup",
                     "SegmentedControl", "DatePicker", "Form", "Timer", "Flashcard", "Carousel",
                     "Popover", "HtmlView", "YouTube":
                    values.append(.scalar(element.prop("label")?.ruleScalar?.string.map(JSONValue.string) ?? .null))
                default:
                    tasks.append(.element(element))
                    tasks.append(contentsOf: element.props.reversed().map { .visit($0.value) })
                }
            case let .visit(value): values.append(value)
            case let .list(count):
                let children = Array(values.suffix(count))
                values.removeLast(children.count)
                values.append(.list(children))
            case var .element(element):
                let props = zip(element.props, values.suffix(element.props.count)).map { prop, value in
                    var plain = prop
                    plain.value = value
                    return plain
                }
                values.removeLast(props.count)
                element.props = props
                values.append(.element(element))
            }
        }
        return values.last ?? .scalar(.null)
    }
}

private struct PopoverSlot: View {
    let inlineItems: [AttributedString]?
    let children: [LayoutChild]
    let path: String

    init(value: PropValue, name: String, path: String) {
        self.path = path
        if case let .string(string)? = value.ruleScalar {
            inlineItems = Self.inline(string, allowsLinks: name == "content")
            children = []
        } else {
            inlineItems = nil
            children = Element.layoutChildren(value, at: name)
        }
    }

    @ViewBuilder var body: some View {
        if let inlineItems {
            ForEach(inlineItems.indices, id: \.self) { index in
                let item = inlineItems[index]
                if let first = item.characters.firstIndex(where: { $0 != " " }),
                   let last = item.characters.lastIndex(where: { $0 != " " }) {
                    Text(AttributedString(item[first...last]))
                }
            }
        } else {
            ForEach(children) { $0.draw(in: path) }
        }
    }

    // Adjacent text nodes share a flex item, including text from a link
    // fragment. Only a rendered element starts a new item.
    private static func inline(_ string: String, allowsLinks: Bool) -> [AttributedString] {
        let whitespace = #/[ \t\r\n\u{c}]+/#.matchingSemantics(.unicodeScalar)
        var items: [AttributedString] = []
        var pendingText = ""
        func flushText() {
            guard !pendingText.isEmpty else { return }
            items.append(AttributedString(pendingText.replacing(whitespace, with: " ")))
            pendingText = ""
        }
        walkInline(string[...], text: { pendingText.append(contentsOf: $0) }, element: { source in
            flushText()
            items.append(formatted(String(source).replacing(whitespace, with: " "), allowsLinks: allowsLinks))
        })
        flushText()
        return items
    }

    private static func walkInline(_ string: Substring, text: (Substring) -> Void, element: (Substring) -> Void) {
        let token = #/`[^`\n]+`|\*\*[^*\n]+?\*\*|__[^_\n]+?__|~~[^~\n]+?~~|\*[^*\s][^*\n]*?\*|_[^_\s][^_\n]*?_|\[/#
        var plain = string.startIndex
        var cursor = plain
        var labelEnd = plain
        var urlEnd = plain
        while let match = string[cursor...].firstMatch(of: token) {
            cursor = match.range.upperBound
            var fragment: Substring?
            if match.output == "[" {
                // All opening brackets before the same closing bracket share
                // a candidate destination. Scan each suffix only once.
                if labelEnd < cursor {
                    labelEnd = string[cursor...].firstIndex { $0 == "]" || $0.isNewline } ?? string.endIndex
                }
                guard cursor < labelEnd, string[labelEnd...].hasPrefix("](") else { continue }
                let urlStart = string.index(labelEnd, offsetBy: 2)
                if urlEnd < urlStart {
                    urlEnd = string[urlStart...].firstIndex { $0 == ")" || $0.isWhitespace } ?? string.endIndex
                }
                guard urlStart < urlEnd, urlEnd < string.endIndex, string[urlEnd] == ")" else { continue }
                let href = string[urlStart..<urlEnd]
                let scheme = href.prefix { $0 != ":" }.lowercased()
                if !["http", "https"].contains(scheme) || !Validation.isURL(String(href)) {
                    fragment = string[cursor..<labelEnd]
                }
                cursor = string.index(after: urlEnd)
            }
            text(string[plain..<match.range.lowerBound])
            if let fragment {
                // A label ends at the first ']', so it cannot contain a complete link.
                walkInline(fragment, text: text, element: element)
            } else {
                element(string[match.range.lowerBound..<cursor])
            }
            plain = cursor
        }
        text(string[plain...])
    }

    private static func formatted(_ string: String, allowsLinks: Bool) -> AttributedString {
        var result = (try? AttributedString(markdown: string,
            options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(string)
        for run in result.runs {
            if let url = run.link, !allowsLinks || !["https", "http"].contains(url.scheme?.lowercased()) {
                result[run.range].link = nil
            }
        }
        return result
    }
}

// Size to the content's ideal width, allowing long text to wrap at the
// viewport limit. A maxWidth frame alone would also expand a one-word panel.
private struct PanelWidth: Layout {
    let limit: CGFloat
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let panel = subviews.first else { return .zero }
        let width = min(limit, panel.sizeThatFits(.unspecified).width)
        return CGSize(width: width, height: panel.sizeThatFits(.init(width: width, height: nil)).height)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: bounds.origin, anchor: .topLeading, proposal: .init(bounds.size))
    }
}

struct LayoutPopover: View {
    let element: Element
    let path: String
    @State private var open = false
    @State private var hover = false
    @FocusState private var triggerFocused: Bool
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.intelligentViewport) private var viewport

    var body: some View {
        let trigger = element.props.first { $0.name == "trigger" }.map(TriggerTask.plain) ?? .scalar(.null)
        let slot = PopoverSlot(value: trigger, name: "trigger", path: path)
        let words = slot.inlineItems.map { $0.map { String($0.characters) }.joined() } ?? triggerText(trigger)
        let label = words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Show details" : words
        Button {
            open.toggle()
        } label: {
            HStack(spacing: 4) {
                slot.allowsHitTesting(false)
            }
            .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity, alignment: .leading)
            .clipContainer()
        }
        .buttonStyle(.plain)
        .focused($triggerFocused)
        .foregroundStyle(hover ? AnyShapeStyle(theme.muted.color) : AnyShapeStyle(.foreground))
        .onHover { hover = $0 }
        .accessibilityLabel(label)
        .background {
            if open {
                ScrollDismissal {
                    open = false
                    triggerFocused = true
                }
            }
        }
        .popover(isPresented: $open, attachmentAnchor: .rect(.bounds), arrowEdge: .top) {
            PopoverPanel(content: element.prop("content") ?? .scalar(.null), path: path, label: label,
                         limit: min(280, max(1, (viewport?.width ?? 304) - 24))) {
                open = false
                triggerFocused = true
            }
        }
    }

    private func triggerText(_ value: PropValue) -> String {
        var pending = [value]
        var parts: [String] = []
        while let next = pending.popLast() {
            switch next {
            case let .list(list): pending.append(contentsOf: list.reversed())
            case let .scalar(.string(s)): parts.append(s)
            case let .scalar(.number(n)): parts.append(JavaScript.numberString(n))
            case let .scalar(.bool(b)): parts.append(String(b))
            case let .element(child):
                for prop in child.props.reversed() where
                    ["text", "children", "title", "description", "items", "code", "tex"].contains(prop.name) {
                    pending.append(prop.value)
                }
            default: break
            }
        }
        return parts.joined(separator: " ")
    }
}

// A scroll anywhere in the source window dismisses the panel. Scrolling
// inside the popover's own window leaves it open.
private struct ScrollDismissal: NSViewRepresentable {
    let close: () -> Void

    final class Observer: NSView {
        var close: (() -> Void)?
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            NotificationCenter.default.removeObserver(self, name: NSView.boundsDidChangeNotification, object: nil)
            if window != nil {
                NotificationCenter.default.addObserver(self, selector: #selector(scrolled),
                    name: NSView.boundsDidChangeNotification, object: nil)
            }
        }
        @objc private func scrolled(_ note: Notification) {
            if let clip = note.object as? NSClipView, let window, clip.window === window { close?() }
        }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }

    func makeNSView(context: Context) -> Observer { Observer() }
    func updateNSView(_ view: Observer, context: Context) { view.close = close }
}

struct PopoverPanel: View {
    private static let font = layoutFont(size: 14, weight: 440)
    let content: PropValue
    let path: String
    let label: String
    let limit: CGFloat
    let close: () -> Void
    @Environment(\.intelligentTheme) private var theme
    private enum Boundary { case first, last }
    @FocusState private var focused: Boundary?
    @State private var entered = false

    private func boundary(_ edge: Boundary) -> some View {
        Color.clear.frame(width: 1, height: 1)
            .focusable(true, interactions: .edit).focusEffectDisabled()
            .focused($focused, equals: edge)
            .accessibilityHidden(true)
            .padding(edge == .first ? .bottom : .top, -1)
    }

    var body: some View {
        PanelWidth(limit: limit) {
            VStack(alignment: .leading, spacing: 0) {
                boundary(.first)
                VStack(alignment: .leading, spacing: 8) {
                    PopoverSlot(value: content, name: "content", path: path)
                }
                boundary(.last)
            }
            .font(Self.font).lineHeight(.exact(points: 21))
            .foregroundStyle(theme.ink.color)
            .padding(.horizontal, 14).padding(.vertical, 12)
        }
        .clipContainer(to: RoundedRectangle(cornerRadius: theme.radiusLg))
        .overlay { RoundedRectangle(cornerRadius: theme.radiusLg).strokeBorder(theme.border.color).allowsHitTesting(false) }
        .presentationCornerRadius(theme.radiusLg)
        .presentationBackground(theme.canvas.color)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
        .defaultFocus($focused, .first)
        .onAppear { focused = .first }
        .onChange(of: focused) { _, new in
            if new == .last || new == .first && entered { close() }
            if new == .first { entered = true }
        }
        .onKeyPress(keys: [.tab], phases: .down) { press in
            if focused == .first && press.modifiers.contains(.shift) {
                close()
                return .handled
            }
            return .ignored
        }
        .onExitCommand(perform: close)
    }
}
