import OpenUINative
import SwiftUI

extension EnvironmentValues {
    @Entry fileprivate var richListTitle = false
}

private extension Element {
    var hasListChildren: Bool {
        if case .list? = prop("children") { true } else { false }
    }
}

struct ListItemDrawing: View {
    let element: Element
    let path: String
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.richListTitle) private var rich
    @Environment(\.font) private var font
    @Environment(\.fontResolutionContext) private var context

    private func isBlock(_ child: Element) -> Bool {
        switch child.typeName {
        case "Badge", "Link", "Icon", "IconButton", "Label", "Checkbox", "Popover": false
        case "Button", "SegmentedControl": child.flag("block")
        case "Math": !child.flag("inline")
        default: true
        }
    }

    var body: some View {
        let em = font?.resolve(in: context).pointSize ?? 16
        VStack(alignment: .leading, spacing: 0) {
            StyledWords(words: InlineText.parse(element.word("title") ?? ""), metrics: TextMetrics(em, weight: rich ? 560 : 400),
                        color: theme.ink)
            if let detail = element.word("description"), !detail.isEmpty {
                StyledWords(words: InlineText.parse(detail), metrics: TextMetrics(em * 0.93, 1.55), color: theme.muted)
                    .padding(.top, 2)
            }
            if case .list? = element.prop("children") {
                ListChildrenLayout {
                    ForEach(element.layoutChildren) { item in
                        switch item.content {
                        case let .element(child):
                            ElementView(element: child, path: ElementPath.child(of: path, item.id))
                                .layoutValue(key: ListBlock.self, value: isBlock(child))
                        case let .text(text): Text(verbatim: text).textSelection(.enabled)
                        }
                    }
                }
                .clipContainer()
                .padding(.top, 8)
            }
        }
        .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
        .clipContainer()
    }
}

private enum ListBlock: LayoutValueKey {
    static let defaultValue = false
}

// The children wrapper is ordinary block flow. Inline children share the
// current line, while each block gets the available width on its own line.
private struct ListChildrenLayout: Layout {
    private func plan(width: CGFloat?, views: Subviews) -> (size: CGSize, frames: [CGRect]) {
        var frames: [CGRect] = []
        var x: CGFloat = 0, y: CGFloat = 0, lineHeight: CGFloat = 0, widest: CGFloat = 0
        var line: [CGRect] = []
        func finishLine() {
            frames.append(contentsOf: line.map { $0.offsetBy(dx: 0, dy: (lineHeight - $0.height) / 2) })
            line.removeAll(keepingCapacity: true)
            widest = max(widest, x)
            y += lineHeight
            x = 0
            lineHeight = 0
        }
        for view in views {
            let block = view[ListBlock.self]
            let ideal = view.sizeThatFits(.unspecified)
            if block || x > 0 && x + ideal.width > (width ?? .infinity) { finishLine() }
            let w = block ? width : min(ideal.width, width ?? ideal.width)
            let size = view.sizeThatFits(ProposedViewSize(width: w, height: nil))
            line.append(CGRect(origin: CGPoint(x: x, y: y), size: size))
            x += size.width
            lineHeight = max(lineHeight, size.height)
            if block { finishLine() }
        }
        finishLine()
        return (CGSize(width: width ?? widest, height: y), frames)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        plan(width: proposal.width.flatMap { $0.isFinite ? layoutPoints($0) : nil }, views: subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (view, frame) in zip(subviews, plan(width: bounds.width, views: subviews).frames) {
            view.place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
                       anchor: .topLeading, proposal: ProposedViewSize(frame.size))
        }
    }
}

private extension TextPart {
    var listItem: Element? { element?.typeName == "ListItem" ? element : nil }
}

struct ListDrawing: View {
    @Environment(\.richListTitle) private var inheritedRich
    let element: Element
    let path: String
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.intelligentViewport) private var viewport
    @Environment(\.font) private var font
    @Environment(\.fontResolutionContext) private var context

    private func isStep(_ custom: String) -> Bool {
        (1...2).contains(custom.utf8.count) && custom.utf8.allSatisfy { (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) }
    }

    private var timeline: Bool { element.flag("timeline") }
    private var numbered: Bool { element.word("marker") == "number" }
    private var unmarked: Bool { element.word("marker") == "none" }
    private var ring: some View {
        Circle().fill(theme.canvas.color).overlay { Circle().strokeBorder(theme.ink.color, lineWidth: 2) }.frame(width: 9, height: 9)
    }

    @ViewBuilder private func marker(_ custom: String, number: Int, wide: Bool, lane: CGFloat, compact: Bool) -> some View {
        if isStep(custom) {
            Text(verbatim: custom).font(layoutFont(size: 12, weight: 560).monospacedDigit()).padding(.horizontal, 5)
                .frame(minWidth: 20, minHeight: 20).foregroundStyle(theme.onInk.color).background(theme.ink.color, in: Capsule())
                .frame(width: numbered ? 24 : nil, height: numbered ? 24 : nil)
                .background(numbered ? theme.ink.color : Color.clear, in: Circle())
        } else if !custom.isEmpty {
            Text(verbatim: custom).font(.system(size: 13).monospacedDigit()).foregroundStyle(theme.muted.color)
                .frame(width: lane, alignment: .leading)
                .overlay(alignment: .topLeading) {
                    if timeline && wide { ring.offset(x: lane + (compact ? 6 : 4), y: 7) }
                }
        } else if numbered {
            Text(verbatim: String(number)).font(.system(size: 13).monospacedDigit())
                .foregroundStyle(theme.onInk.color).frame(width: 24, height: 24).background(theme.ink.color, in: Circle())
        } else if !unmarked {
            if timeline { ring } else { Circle().fill(theme.faint.color).frame(width: 5, height: 5) }
        }
    }

    var body: some View {
        let em = font?.resolve(in: context).pointSize ?? 16
        let items = TextPart.parts(element, key: "items", path: path)
        let markers = items.compactMap { $0.listItem?.word("marker") }
        let wide = markers.contains { $0.utf16.count > 2 }
        let chips = markers.contains(where: isStep)
        let compact = viewport.map { $0.width <= 640 } ?? false
        let stack = unmarked && !wide && !chips
        let dots = !wide && !chips && !unmarked && !numbered && !timeline
        let lane: CGFloat = wide ? (compact ? 64 : 76) : dots ? 14 : 24
        let gap: CGFloat = wide ? (compact ? 20 : timeline ? 24 : 16) : dots ? 8 : 12
        let rows = items.enumerated().filter { if case .scalar(.null) = $0.element.value { false } else { true } }
        VStack(alignment: .leading, spacing: timeline ? 0 : 10) {
            ForEach(rows, id: \.element.id) { index, part in
                let child = part.listItem
                let custom = child?.word("marker") ?? ""
                let rich = child?.word("description")?.isEmpty == false || child?.hasListChildren == true
                let last = part.id == rows.last?.element.id
                let layout = stack ? AnyLayout(VStackLayout(alignment: .leading, spacing: 0))
                    : AnyLayout(HStackLayout(alignment: .top, spacing: gap))
                layout {
                    if !custom.isEmpty || !unmarked {
                        marker(custom, number: index + 1, wide: wide, lane: lane, compact: compact)
                            .frame(width: lane, alignment: wide && !isStep(custom) ? .leading : .center)
                            .frame(minHeight: em * 1.55).textSelection(.enabled).accessibilityHidden(custom.isEmpty)
                    }
                    Group {
                        if let child {
                            ElementView(element: child, path: part.id).environment(\.richListTitle, rich || inheritedRich)
                        } else {
                            StyledWords(words: InlineText.parse(part.string), metrics: TextMetrics(em), color: theme.ink)
                        }
                    }
                    // In a two-column grid a display:none marker leaves the
                    // body as the first cell, including when another row sets a wide lane.
                    .frame(width: !stack && unmarked && custom.isEmpty ? lane : nil, alignment: .leading)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.bottom, timeline ? (last ? 0 : 18) : rich ? 4 : 0)
                .background(alignment: .leading) {
                    if timeline && !last && (!unmarked || wide) {
                        Rectangle().fill(theme.borderStrong.color).frame(width: 1)
                            .padding(.top, 1.7 * em).padding(.bottom, 0.15 * em)
                            .offset(x: wide ? lane + (compact ? 10 : 8) - 1 : 11.5)
                    }
                }
            }
        }
        .clipContainer()
    }
}
