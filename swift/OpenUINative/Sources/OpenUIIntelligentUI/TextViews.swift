import OpenUINative
import SwiftUI

struct TextMetrics {
    var size: CGFloat
    var height: CGFloat?
    var weight: CGFloat = 400
    var tracking: CGFloat = 0

    init(_ size: CGFloat, _ factor: CGFloat? = nil, weight: CGFloat = 400, tracking: CGFloat = 0) {
        self.size = size
        height = factor.map { size * $0 }
        self.weight = weight
        self.tracking = tracking
    }

    static func title(_ size: String?, compact: Bool) -> Self {
        switch size {
        case "sm": Self(15, 1.5, weight: 640)
        case "lg": Self(compact ? 19 : 21, 1.3, weight: 640, tracking: -0.55)
        case "xl": Self(25, 1.2, weight: 640, tracking: -0.8)
        case "2xl": Self(compact ? 28 : 32, 1.15, weight: 640, tracking: -1)
        case "3xl": Self(compact ? 34 : 40, 1.1, weight: 640, tracking: -1.4)
        default: Self(17, 1.4, weight: 640, tracking: -0.2)
        }
    }
}

extension Element {
    func word(_ name: String) -> String? { prop(name)?.ruleScalar?.string }
    func flag(_ name: String) -> Bool { prop(name)?.ruleScalar == .bool(true) }
}

struct WordStyle {
    var metrics: TextMetrics
    var color: ThemeColor
    var italic = false
    var strike = false
    var tabular = false
    var strong: CGFloat = 640

    func font(for run: AttributedString.Runs.Run) -> Font {
        let intent = run.inlinePresentationIntent ?? []
        let weight = intent.contains(.stronglyEmphasized) ? strong : metrics.weight
        let code = intent.contains(.code)
        var font = run.font ?? layoutFont(size: code ? metrics.size * 0.88 : metrics.size, weight: weight)
        if code { font = font.monospaced() }
        if italic || intent.contains(.emphasized) { font = font.italic() }
        if tabular { font = font.monospacedDigit() }
        return font
    }
}

extension EnvironmentValues {
    @Entry fileprivate var paragraphType: WordStyle? = nil
}

struct StyledWords: View {
    var words: AttributedString
    var style: WordStyle
    var preserveLines: Bool? = false
    @Environment(\.intelligentTheme) private var theme

    @Environment(\.lineHeight) private var inheritedHeight

    init(_ words: AttributedString, style: WordStyle, preserveLines: Bool? = false) {
        self.words = words
        self.style = style
        self.preserveLines = preserveLines
    }

    init(words: AttributedString, metrics: TextMetrics, color: ThemeColor, strong: CGFloat = 640) {
        self.init(words, style: WordStyle(metrics: metrics, color: color, strong: strong))
    }

    var body: some View {
        let visible = preserveLines.map { InlineText.whitespace(words, preserveLines: $0) } ?? words
        var styled = visible
        for run in visible.runs {
            let intent = run.inlinePresentationIntent ?? []
            if intent.contains(.code) { styled[run.range].backgroundColor = theme.surface.color }
            styled[run.range].inlinePresentationIntent = nil
            styled[run.range].font = style.font(for: run)
            if style.strike || intent.contains(.strikethrough) { styled[run.range].strikethroughStyle = .single }
            if run.link != nil { styled[run.range].underlineStyle = .init(pattern: .solid, color: theme.borderStrong.color) }
        }
        return Text(styled)
            .tracking(style.metrics.tracking)
            .foregroundStyle(style.color.color)
            .tint(style.color.color)
            .lineHeight(style.metrics.height.map { .exact(points: $0) } ?? inheritedHeight)
            .textSelection(.enabled)
            .frame(height: visible.characters.isEmpty ? 0 : nil)
    }
}

struct TextLeaf: View {
    let element: Element
    let path: String
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.intelligentViewport) private var viewport
    @Environment(\.cardTypography) private var card
    @Environment(\.paragraphType) private var inherited
    @Environment(\.font) private var font
    @Environment(\.fontResolutionContext) private var context
    @Environment(\.colorScheme) private var scheme

    private var metrics: TextMetrics {
        let size = element.word("size")
        let em = font?.resolve(in: context).pointSize ?? 16
        var result: TextMetrics
        switch element.typeName {
        case "Title": result = .title(size, compact: viewport.map { $0.width <= 640 } ?? false)
        case "Caption": result = TextMetrics(12, 1.6)
        case "Label": result = TextMetrics(16, 1.5, weight: scheme == .dark ? 420 : 440, tracking: -0.16)
        case "Link": result = inherited?.metrics ?? TextMetrics(em)
        default:
            switch size {
            case "xs": result = TextMetrics(12, 1.6)
            case "sm": result = TextMetrics(14, 1.55)
            case "md": result = TextMetrics(16, 1.6)
            case "lg": result = TextMetrics(18, 1.55)
            default: result = TextMetrics(inherited?.metrics.size ?? em, 1.6)
            }
            result.weight = inherited?.metrics.weight ?? result.weight
        }
        if let card {
            result.size = card.font.resolve(in: context).pointSize
            result.height = card.lineHeight
            result.tracking = card.tracking ?? result.tracking
            if element.typeName == "Title", result.size == 14 { result.weight = 560 }
        }
        guard element.typeName == "Text" else { return result }
        switch element.word("weight") {
        case "normal": result.weight = 440
        case "medium": result.weight = 560
        case "semibold": result.weight = 640
        default: break
        }
        return result
    }

    private var color: ThemeColor {
        let parentInk = inherited?.color ?? theme.ink
        if element.typeName == "Link" { return parentInk }
        guard ["Title", "Text", "Caption"].contains(element.typeName) else { return theme.ink }
        let paint = ColorValue.foreground(element.prop("color")).flatMap(theme.paint)
        switch paint {
        case let .color(color): return color
        case .currentColor: return parentInk
        case nil:
            if element.typeName == "Caption" || card?.muted == true { return theme.muted }
            return element.typeName == "Text" ? parentInk : theme.ink
        }
    }

    private var maxLines: Int? {
        guard case let .number(n)? = element.prop("maxLines")?.ruleScalar, n.isFinite, n.rounded() >= 1 else { return nil }
        return Int(exactly: n.rounded())
    }

    private var style: WordStyle {
        let paragraph = element.typeName == "Text"
        return WordStyle(metrics: metrics, color: color,
                         italic: inherited?.italic == true || paragraph && element.flag("italic"),
                         strike: inherited?.strike == true || paragraph && element.flag("lineThrough"),
                         tabular: inherited?.tabular == true || element.typeName == "Title" || paragraph && element.flag("tabularNums"),
                         strong: element.typeName == "Title" ? 900 : 640)
    }

    private func styled(_ value: AttributedString, preserveLines: Bool? = false) -> StyledWords {
        StyledWords(value, style: style, preserveLines: preserveLines)
    }

    var body: some View {
        let string = element.word("text") ?? ""
        switch element.typeName {
        case "Title":
            let size = element.word("size") ?? "md"
            styled(InlineText.parse(string)).accessibilityHeading(size == "sm" ? .h4 : size == "md" ? .h3 : size == "3xl" ? .h1 : .h2)
        case "Label":
            styled(AttributedString(string))
        case "Link":
            styled(linkLabel(string))
        case "Text":
            if case .list? = element.prop("text") {
                let parts = TextPart.parts(element, key: "text", path: path)
                if parts.contains(where: { $0.element != nil }) {
                    let type = style
                    InlineParagraph(parts: parts, style: type, limit: maxLines)
                    .font(layoutFont(size: type.metrics.size, weight: type.metrics.weight))
                    .environment(\.cardTypography, nil)
                    .environment(\.paragraphType, type)
                } else {
                    let joined = parts.reduce(into: AttributedString()) { $0.append(InlineText.parse($1.string)) }
                    styled(joined, preserveLines: true).lineLimit(maxLines)
                }
            } else {
                styled(InlineText.parse(string), preserveLines: true).lineLimit(maxLines)
            }
        default:
            styled(InlineText.parse(string))
        }
    }

    private func linkLabel(_ label: String) -> AttributedString {
        guard let url = webURL(element.word("url") ?? "") else { return AttributedString(label) }
        let host = URLComponents(url: url, resolvingAgainstBaseURL: false)?.encodedHost ?? ""
        var text = AttributedString(label.isEmpty ? host : label)
        var arrow = AttributedString(" ↗")
        arrow.font = .system(size: metrics.size * 0.85)
        text.append(arrow)
        text.link = url
        return text
    }
}
