// Child views always go back through ElementView, including for a bare tree
// whose children need explicit paths. A flex parent owns its children's
// border-box sizes; outside one a Box resolves its own width and height.
import OpenUINative
import SwiftUI

struct LayoutChild: Identifiable {
    enum Content {
        case element(Element)
        case text(String)
    }

    let id: String
    let content: Content
    let previous: Element?

    var element: Element? {
        if case let .element(child) = content { child } else { nil }
    }
}

extension LayoutChild {
    // Unwrapping a Pressable in a popover trigger moves its children in the
    // drawn tree. Keep their original live paths just as ElementView does.
    func path(in parent: String) -> String { element?.path ?? ElementPath.child(of: parent, id) }

    @MainActor func draw(in parent: String) -> some View {
        Group {
            switch content {
            case let .text(string): Text(verbatim: string)
            case let .element(child): ElementView(element: child, path: path(in: parent))
            }
        }
        .transformAnchorPreference(key: CarouselBounds.self, value: .bounds) { bounds, anchor in
            // CSS contains scroll overflow at nested scrollports and at Boxes
            // with explicit radius. Their own bounds still reach the track.
            if let element, element.typeName == "Carousel" || element.typeName == "Box" && element.prop("radius") != nil {
                bounds.removeAll()
            }
            bounds.append(anchor)
        }
    }

    @MainActor func drawCell(in parent: String, stretches: Bool) -> some View {
        draw(in: parent).environment(\.trackCell, TrackCell(path: path(in: parent), stretches: stretches))
    }
}

// A grid stretches an auto-sized child in its row. A carousel item only
// stretches a direct Card or Pressable. Both scale type in a direct flow.
struct TrackCell: Sendable {
    let path: String
    let stretches: Bool
}

extension Element {
    func prop(_ name: String) -> PropValue? {
        props.first { $0.name == name }?.value
    }

    // renderDeep flattens fragments. CSS collects adjacent DOM text nodes in
    // one anonymous flex item. Element sibling selectors skip those text nodes.
    var layoutChildren: [LayoutChild] {
        guard let value = prop("children") else { return [] }
        return Self.layoutChildren(value, at: "children")
    }

    static func layoutChildren(_ value: PropValue, at path: String) -> [LayoutChild] {
        var pending = [(value, path)]
        var children: [LayoutChild] = []
        var previous: Element?
        var text = ""
        var textPath = ""
        func flushText() {
            let words = text.unicodeScalars.split { " \t\n\r\u{0c}".unicodeScalars.contains($0) }
            let collapsed = words.map(String.init).joined(separator: " ")
            if !collapsed.isEmpty {
                children.append(LayoutChild(id: textPath, content: .text(collapsed), previous: previous))
            }
            text = ""
        }
        while let (value, path) = pending.popLast() {
            switch value {
            case let .list(list):
                for (index, item) in list.enumerated().reversed() {
                    pending.append((item, ElementPath.child(of: path, String(index))))
                }
            case let .element(child):
                if child.typeName == "Chart", ChartRows.hasNoRowsOrCategory(child) { continue }
                flushText()
                children.append(LayoutChild(id: path, content: .element(child), previous: previous))
                previous = child
            case let .scalar(primitive):
                if text.isEmpty { textPath = path }
                switch primitive {
                case let .string(string): text += string
                case let .number(n): text += JavaScript.numberString(n)
                case let .bool(b): text += String(b)
                default: break
                }
            default: continue
            }
        }
        flushText()
        return children
    }

    fileprivate var hasBorder: Bool { typeName == "Card" || prop("border")?.ruleScalar == .bool(true) }

    fileprivate func inset(nestedCard: Bool, compact: Bool) -> CGFloat {
        let standard: CGFloat = typeName == "Card" ? (nestedCard || compact ? 16 : 20) : 0
        return layoutPoints(Spacing.points(prop("padding")) ?? standard) + (hasBorder ? 1 : 0)
    }

    fileprivate var smallTitle: Bool {
        typeName == "Title" && ["sm", "md"].contains(prop("size")?.ruleScalar?.string ?? "md")
    }

    fileprivate var row: Bool {
        typeName == "Row" || typeName == "Box" && prop("direction")?.ruleScalar?.string == "row"
    }
}

extension EnvironmentValues {
    @Entry var intelligentViewport: CGSize? = nil
    // A leaf which sets its own font or line height reads these overrides.
    @Entry var cardTypography: CardTypography?
    @Entry var trackCell: TrackCell?
    @Entry fileprivate var insideFlow = false
    @Entry fileprivate var insideCard = false
    @Entry fileprivate var flexChildPath: String? = nil
    @Entry fileprivate var definiteFlexHeight = false
    @Entry var inOpeningRow = false
    @Entry fileprivate var photoBox = false
}

extension View {
    /// Apply after sizing the container, before adding any outer chrome.
    func clipContainer<S: Shape>(to bounds: S = Rectangle()) -> some View {
        // Keep the clipped subtree in its own bounded compositing group. On
        // macOS this also groups embedded AppKit views at the clip boundary.
        clipShape(bounds).contentShape(.interaction, bounds).compositingGroup()
    }
}

public extension View {
    /// The viewport for a windowless export. Apply at the root with the
    /// export dimensions so vw, vh and compact Card sizing work on first draw.
    func intelligentUIViewport(_ size: CGSize) -> some View {
        environment(\.intelligentViewport, size)
    }
}

struct LayoutContainer: View {
    let element: Element
    let path: String
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.intelligentInk) private var inheritedInk
    @Environment(\.intelligentViewport) private var viewport
    @State private var rootViewport: CGSize?

    init(element: Element, path: String) {
        // Model-supplied element objects can contain props outside the schema.
        // Mirror the props forwarded by each lib wrapper to its styled view.
        var panel = element
        switch element.typeName {
        case "Col": panel.props.removeAll { !["children", "gap", "align"].contains($0.name) }
        case "Row": panel.props.removeAll { !["children", "gap", "align", "justify", "wrap"].contains($0.name) }
        case "Card": panel.props.removeAll { !["children", "gap", "variant", "padding"].contains($0.name) }
        default: break // Box forwards every layout prop.
        }
        self.element = panel
        self.path = path
    }

    var body: some View {
        let background = element.typeName == "Box" ? element.prop("background")?.ruleScalar : nil
        let raw = ColorValue.background(background)
        let contrast = background?.string.flatMap(onColor)
        // Read the theme below the override, so backgrounds and descendant
        // tokens that depend on ink see the same text colour.
        let panel = LayoutPanel(element: element, path: path, background: raw)
            .environment(\.intelligentInk, contrast ?? inheritedInk)
            .environment(\.intelligentViewport, viewport ?? rootViewport)
            .background {
                if path == ElementPath.root && viewport == nil {
                    Color.clear.containerRelativeFrame([.horizontal, .vertical])
                        .onGeometryChange(for: CGSize.self, of: { $0.size }) { rootViewport = $0 }
                }
            }
        if element.typeName == "Col", element.statementId == "root" {
            panel.fontDesign(theme.font).intelligentForegroundStyle(theme.ink)
        } else if let contrast {
            panel.intelligentForegroundStyle(contrast)
        } else {
            panel
        }
    }
}

private struct LayoutPanel: View {
    let element: Element
    let path: String
    let background: ColorValue?
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.inOpeningRow) private var inOpeningRow
    @Environment(\.intelligentViewport) private var viewport
    @Environment(\.trackCell) private var trackCell
    @Environment(\.insideFlow) private var nestedFlow
    @Environment(\.insideCard) private var nestedCard
    @Environment(\.definiteFlexHeight) private var definiteFlexHeight
    @Environment(\.flexChildPath) private var flexChildPath
    @Environment(\.photoBox) private var photoBox
    @Environment(\.font) private var font
    @Environment(\.fontResolutionContext) private var fontContext
    @Environment(\.multilineTextAlignment) private var textAlignment

    private var card: Bool { element.typeName == "Card" }
    private var flow: Bool { !element.row && (element.prop("gap")?.ruleScalar ?? .null) == .null }

    private func margin(after previous: Element?, before child: Element) -> CGFloat {
        guard flow, let previous else { return 0 }
        let title = previous.typeName == "Title"
        if title && (child.typeName == "Text" || child.typeName == "Caption"
            || child.row && child.layoutChildren.contains { $0.element?.typeName == "Badge" }) { return theme.spaceInner }
        if previous.typeName == "Caption" && child.typeName == "Title" { return theme.spaceTight }
        if title || nestedFlow || card || previous.typeName == "Text" && child.typeName == "Text" { return theme.spaceGroup }
        return theme.spaceSection
    }

    private func item(_ child: Element, previous: Element?) -> FlexItem {
        var item = FlexItem()
        item.nonwrappingCarousel = child.typeName == "Carousel"
        item.margin = margin(after: previous, before: child)
        if child.typeName == "Badge" { item.selfAlignment = element.typeName == "Row" ? .center : .start }
        if element.row && child.typeName == "Button" { item.selfAlignment = .center }
        if element.typeName == "Row" && child.typeName == "Badge" {
            item.fixed = true
            item.maxWidth = .percent(100)
        }
        if child.typeName == "Icon" {
            let side = Double(IconDrawing.side(for: child))
            item.width = .points(side)
            item.height = .points(side)
            item.fixed = true
        }
        item.photo = child.typeName == "Image"
        item.boxedPhoto = photoBox && item.photo
        if child.typeName == "Divider" || child.typeName == "Card" { item.width = .percent(100) }
        if child.typeName == "HtmlView" { item.width = .percent(100) }
        if child.typeName == "Box" || child.typeName == "Card" {
            item.minimum = 2 * child.inset(nestedCard: nestedCard || card, compact: viewport.map { $0.width <= 640 } ?? false)
        }
        if child.typeName == "Divider" { item.height = .points(1) }
        if child.typeName == "Spacer" { item.grow = 1 }
        if child.typeName == "Box" {
            item.wrapsColumn = !child.row && child.prop("wrap")?.ruleScalar == .bool(true)
            item.width = Length(child.prop("width"))
            item.height = Length(child.prop("height"))
            if case let .number(n)? = child.prop("flex")?.ruleScalar, n.isFinite {
                item.grow = min(Double(Float.greatestFiniteMagnitude), max(0, n))
            } else if case .number? = child.prop("width")?.ruleScalar {
                item.fixed = true
            }
            let elements = child.layoutChildren.compactMap(\.element)
            item.photoBox = elements.count == 1 && elements.first?.typeName == "Image"
        }
        return item
    }

    var body: some View {
        let e = element
        let children = e.layoutChildren
        let second = children.lazy.filter { $0.element != nil }.dropFirst().first?.id
        let media = e.row && children.contains { $0.element?.typeName == "Image" }
        let align = media ? FlexAlign.start : e.prop("align")?.ruleScalar?.string.flatMap(FlexAlign.init)
            ?? (e.row ? .center : .stretch)
        let gap = media ? theme.spaceGroupLg : Spacing.points(e.prop("gap")) ?? (e.row ? theme.spaceInner : 0)
        let border = e.hasBorder
        let soft = card && e.prop("variant")?.ruleScalar == .string("soft")
        let radius = card ? (nestedCard ? theme.radiusLg : theme.radiusXl)
            : Radius(e.prop("radius")).map(theme.radius)
        let flexChild = flexChildPath == path
        let cell = trackCell?.path == path
        let cardSizedType = card || (cell && flow)
        let em: CGFloat = card ? 14 : font?.resolve(in: fontContext).pointSize ?? 16
        let stretchedCell = cell && trackCell?.stretches == true
        let definiteHeight = flexChild ? definiteFlexHeight
            : stretchedCell || Length(e.prop("height"))?.isDefinite(in: true) == true
        let layout = FlexLayout(row: e.row, align: align,
                                justify: e.prop("justify")?.ruleScalar?.string.flatMap(FlexJustify.init) ?? .start,
                                wrap: e.prop("wrap")?.ruleScalar == .bool(true), gap: layoutPoints(gap),
                                inset: e.inset(nestedCard: nestedCard, compact: false), viewport: viewport, em: em,
                                acceptsHeight: flexChild || stretchedCell, definiteHeight: definiteHeight,
                                narrowInset: card ? e.inset(nestedCard: nestedCard, compact: true) : nil,
                                ownWidth: flexChild ? nil : Length(e.prop("width")),
                                ownHeight: flexChild ? nil : Length(e.prop("height")))
        let shape = RoundedRectangle(cornerRadius: layoutPoints(radius ?? 0), style: .circular)
        let fill = card ? CSSPaint.color(soft ? theme.surface : theme.canvas)
            : background.flatMap(theme.paint)
        let content = layout {
            ForEach(children) { entry in
                switch entry.content {
                case .text:
                    entry.draw(in: path)
                case let .element(child):
                    let previous = entry.previous
                    let openingTitle = child.smallTitle && (card && previous == nil || inOpeningRow)
                    let subtitle = (child.typeName == "Text" || child.typeName == "Caption")
                        && previous?.smallTitle == true && (card && entry.id == second || inOpeningRow)
                    let item = item(child, previous: previous)
                    // The post-flex main size is definite in a definite column.
                    // A stretched auto cross size is definite for its descendants.
                    let childHeight = item.height?.isDefinite(in: definiteHeight) == true
                    let stretches = e.row && item.stretches(inRow: true, align: align)
                    let definite = childHeight || (!e.row && definiteHeight) || stretches || e.row && item.photo
                    let typography = CardTypography(child: child, direct: cardSizedType, opener: openingTitle, subtitle: subtitle)
                    entry.draw(in: path)
                        .modifier(CardText(typography: typography))
                        .environment(\.cardTypography, typography)
                        .padding(.top, subtitle && !flow ? 2 : 0)
                        .environment(\.inOpeningRow, inOpeningRow || card && previous == nil && child.row)
                        .environment(\.flexChildPath, entry.path(in: path))
                        .environment(\.definiteFlexHeight, definite)
                        .environment(\.photoBox, e.row && item.photoBox)
                        .layoutValue(key: FlexItem.self, value: item)
                }
            }
        }
        .environment(\.insideFlow, flow || nestedFlow)
        .environment(\.insideCard, card || nestedCard)
        .multilineTextAlignment(!e.row && align == .end ? .trailing :
                                !e.row && align == .center ? .center : textAlignment)
        .clipContainer(to: shape.inset(by: border ? 1 : 0))
        .background {
            switch fill {
            case let .color(color): shape.fill(color.color)
            case .currentColor: shape.fill(.foreground)
            case nil: EmptyView()
            }
        }
        .overlay { if border && !soft { shape.strokeBorder(theme.border.color, lineWidth: 1) } }
        if card {
            content.font(.system(size: 14)).lineHeight(.multiple(factor: 1.55))
        } else {
            content
        }
    }
}

// The theme uses the system variable font with the requested CSS weight.
func layoutFont(size: CGFloat, weight: CGFloat) -> Font {
    Font(layoutPlatformFont(size: size, weight: weight))
}

func layoutPlatformFont(size: CGFloat, weight: CGFloat) -> CTFont {
    let system = NSFont.systemFont(ofSize: size)
    let descriptor = CTFontDescriptorCreateCopyWithVariation(
        CTFontCopyFontDescriptor(system), 0x77676874 as CFNumber, weight
    )
    return CTFontCreateWithFontDescriptor(descriptor, size, nil)
}

// A leading Row also opens a Card. Direct default/md Text keeps the
// card-sized 15/22 body metrics even when it is a muted subtitle.
struct CardTypography: Sendable {
    private static let openingFont = layoutFont(size: 14, weight: 560)
    private static let largeFont = layoutFont(size: 18, weight: 640)

    let font: Font
    let lineHeight: CGFloat
    let muted: Bool
    let tracking: CGFloat?

    init?(child: Element, direct: Bool, opener: Bool, subtitle: Bool) {
        let size = child.prop("size")?.ruleScalar?.string
        let bodyText = direct && child.typeName == "Text" && (size == nil || size == "md")
        muted = subtitle
        if opener {
            (font, lineHeight, tracking) = (Self.openingFont, 20, nil)
        } else if bodyText {
            (font, lineHeight, tracking) = (.system(size: 15), 22, nil)
        } else if subtitle {
            (font, lineHeight, tracking) = (.system(size: 13), 18, nil)
        } else if direct && child.typeName == "Title" && !child.smallTitle {
            (font, lineHeight, tracking) = (Self.largeFont, 24, -0.3)
        } else {
            return nil
        }
    }
}

private struct CardText: ViewModifier {
    let typography: CardTypography?
    @Environment(\.intelligentTheme) var theme

    @ViewBuilder func body(content: Content) -> some View {
        if let typography {
            let text = content.font(typography.font).lineHeight(.exact(points: typography.lineHeight))
            if typography.muted { text.intelligentForegroundStyle(theme.muted) }
            else if let tracking = typography.tracking { text.tracking(tracking) }
            else { text }
        } else {
            content
        }
    }
}

struct LayoutDivider: View {
    @Environment(\.colorScheme) var scheme
    var body: some View {
        IntelligentTheme.separator(scheme).color
            .frame(minHeight: 0, idealHeight: 1, maxHeight: 1)
            .alignmentGuide(.flexContentHeight) { _ in 0 }
    }
}

struct LayoutSpacer: View {
    var body: some View { Color.clear.frame(idealWidth: 0, idealHeight: 0).accessibilityHidden(true) }
}
