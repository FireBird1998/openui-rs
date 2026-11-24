import OpenUINative
import SwiftUI

// lib/Text and lib/List flatten their array prop one level. Keep the actual
// indices through that flatten, including for a detached ElementView.
struct TextPart: Identifiable {
    let id: String
    let value: PropValue

    static func parts(_ element: Element, key: String, path: String) -> [Self] {
        guard case let .list(items)? = element.prop(key) else { return [] }
        let base = ElementPath.child(of: path, key)
        return items.enumerated().flatMap { i, value -> [Self] in
            let item = ElementPath.child(of: base, String(i))
            if case let .list(nested) = value {
                return nested.enumerated().map { Self(id: ElementPath.child(of: item, String($0.offset)), value: $0.element) }
            }
            return [Self(id: item, value: value)]
        }
    }

    var element: Element? { if case let .element(child) = value { child } else { nil } }

    var string: String {
        var result = ""
        var pending = [ArraySlice([value])]
        while let rest = pending.popLast(), let item = rest.first {
            if rest.startIndex > 0 { result.append(",") }
            if rest.count > 1 { pending.append(rest.dropFirst()) }
            switch item {
            case let .scalar(json): result += (try? ListFunctions.text(json)) ?? ""
            case let .list(items):
                if !items.isEmpty { pending.append(items[...]) }
            default: result += "[object Object]"
            }
        }
        return result
    }
}

// Each element occupies one object-replacement character in the attributed
// paragraph. Core Text breaks and reorders the whole line, including its slots.
private enum InlineSlot: AttributedStringKey {
    static let name = "OpenUI.InlineSlot"
    typealias Value = Int
}

private final class InlineContent: Sendable {
    let words: AttributedString
    let fonts: [(range: NSRange, font: Font.Resolved)]
    let tracking: CGFloat
    let slots: [(offset: Int, child: Int)]
    let children: [TextPart]
    let lineHeight: CGFloat
    let limit: Int?
    let alignment: TextAlignment
    let direction: LayoutDirection

    init(_ parts: [TextPart], style: WordStyle, context: Font.Context, limit: Int?,
         alignment: TextAlignment, direction: LayoutDirection) {
        self.limit = limit
        self.alignment = alignment
        self.direction = direction
        var joined = AttributedString()
        var children: [TextPart] = []
        for part in parts {
            if part.element != nil {
                var slot = AttributedString("\u{fffc}")
                slot[InlineSlot.self] = children.count
                joined.append(slot)
                children.append(part)
            } else {
                joined.append(InlineText.parse(part.string))
            }
        }
        let words = InlineText.whitespace(joined, preserveLines: true)
        var slots: [(Int, Int)] = []
        var fonts: [(NSRange, Font.Resolved)] = []
        var offset = 0
        for run in words.runs {
            if let child = run[InlineSlot.self] { slots.append((offset, child)) }
            let length = words[run.range].utf16.count
            fonts.append((NSRange(location: offset, length: length), style.font(for: run).resolve(in: context)))
            offset += length
        }
        self.fonts = fonts
        self.tracking = style.metrics.tracking
        self.words = words
        self.children = children
        self.slots = slots
        lineHeight = style.metrics.height ?? style.metrics.size * 1.6
    }

    private static func delegate(width: CGFloat) -> CTRunDelegate? {
        var callbacks = CTRunDelegateCallbacks(version: kCTRunDelegateCurrentVersion,
            dealloc: { Unmanaged<NSNumber>.fromOpaque($0).release() },
            getAscent: { _ in 0 }, getDescent: { _ in 0 },
            getWidth: { CGFloat(Unmanaged<NSNumber>.fromOpaque($0).takeUnretainedValue().doubleValue) })
        let pointer = Unmanaged.passRetained(NSNumber(value: Double(width))).toOpaque()
        let delegate = CTRunDelegateCreate(&callbacks, pointer)
        if delegate == nil { Unmanaged<NSNumber>.fromOpaque(pointer).release() }
        return delegate
    }

    struct Fragment {
        let range: NSRange
        var rect: CGRect
    }

    struct Plan {
        var size: CGSize = .zero
        var text: [Fragment] = []
        var children: [Int: CGRect] = [:]
    }

    func plan(width: CGFloat?, sizes: [Int: CGSize]) -> Plan {
        let source = NSMutableAttributedString(attributedString: NSAttributedString(words))
        let paragraph = NSMutableParagraphStyle()
        paragraph.baseWritingDirection = direction == .rightToLeft ? .rightToLeft : .leftToRight
        source.addAttributes([.paragraphStyle: paragraph, .kern: tracking], range: NSRange(location: 0, length: source.length))
        for (range, font) in fonts { source.addAttribute(.font, value: font.ctFont, range: range) }
        let childKey = NSAttributedString.Key(InlineSlot.name)
        let segmentKey = NSAttributedString.Key("OpenUI.ProseSegment")
        var segmentStart = 0
        for (offset, child) in slots {
            source.addAttribute(segmentKey, value: segmentStart, range: NSRange(location: segmentStart, length: offset - segmentStart))
            segmentStart = offset + 1
            let range = NSRange(location: offset, length: 1)
            if let delegate = Self.delegate(width: sizes[child]?.width ?? 0) {
                source.addAttribute(NSAttributedString.Key(kCTRunDelegateAttributeName as String), value: delegate, range: range)
            }
            source.addAttribute(childKey, value: child, range: range)
        }
        source.addAttribute(segmentKey, value: segmentStart, range: NSRange(location: segmentStart, length: source.length - segmentStart))
        guard let typesetter = CTTypesetterCreateWithAttributedStringAndOptions(source, nil) else { return Plan() }
        let maximum = width ?? layoutPoints(.infinity)
        var result = Plan()
        var start = 0, lineCount = 0
        while start < source.length && lineCount < (limit ?? .max) {
            let suggested = CTTypesetterSuggestLineBreak(typesetter, start, maximum)
            let count = suggested > 0 ? suggested : (source.string as NSString).rangeOfComposedCharacterSequence(at: start).length
            let line = CTTypesetterCreateLine(typesetter, CFRange(location: start, length: count))
            let extent = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil) - CTLineGetTrailingWhitespaceWidth(line))
            let extra = max(0, (width ?? extent) - extent)
            let atRight = (alignment == .trailing) != (direction == .rightToLeft)
            var x: CGFloat = alignment == .center ? extra / 2 : atRight ? extra : 0
            let y = result.size.height
            var height = lineHeight
            var rowChildren: [(Int, CGRect)] = []
            var rowText: [Fragment] = []
            var fragment: Fragment?
            var previousSegment: Int?
            // Core Text returns runs in visual order. Keep contiguous prose
            // together for selection, styled links, and bidirectional shaping.
            for run in CTLineGetGlyphRuns(line) as? [CTRun] ?? [] {
                let advance = CGFloat(CTRunGetTypographicBounds(run, CFRange(), nil, nil, nil))
                let range = CTRunGetStringRange(run)
                let rect = CGRect(x: x, y: y, width: advance, height: lineHeight)
                let attributes = CTRunGetAttributes(run) as NSDictionary
                if let child = attributes[childKey] as? Int {
                    let size = sizes[child] ?? .zero
                    if let fragment { rowText.append(fragment) }
                    fragment = nil
                    height = max(height, size.height)
                    rowChildren.append((child, CGRect(origin: rect.origin, size: size)))
                } else {
                    let chars = NSRange(location: range.location, length: range.length)
                    let segment = attributes[segmentKey] as? Int
                    if let previous = fragment, segment == previousSegment {
                        fragment = Fragment(range: NSUnionRange(previous.range, chars), rect: previous.rect.union(rect))
                    } else {
                        if let fragment { rowText.append(fragment) }
                        fragment = Fragment(range: chars, rect: rect)
                    }
                    previousSegment = segment
                }
                x += advance
            }
            if let fragment { rowText.append(fragment) }
            for var text in rowText {
                text.rect.origin.y += (height - lineHeight) / 2
                result.text.append(text)
            }
            for (child, frame) in rowChildren {
                result.children[child] = frame.offsetBy(dx: 0, dy: (height - frame.height) / 2)
            }
            result.size.height += height
            result.size.width = max(result.size.width, extent)
            start += count
            lineCount += 1
        }
        if let width { result.size.width = width }
        return result
    }
}

private struct InlineBounds: PreferenceKey {
    static let defaultValue: [Int: Anchor<CGRect>] = [:]
    static func reduce(value: inout [Int: Anchor<CGRect>], nextValue: () -> [Int: Anchor<CGRect>]) {
        value.merge(nextValue(), uniquingKeysWith: { _, latest in latest })
    }
}

struct InlineParagraph: View {
    let parts: [TextPart]
    let style: WordStyle
    let limit: Int?
    @Environment(\.multilineTextAlignment) private var alignment
    @Environment(\.fontResolutionContext) private var context
    @Environment(\.layoutDirection) private var direction

    var body: some View {
        let content = InlineContent(parts, style: style, context: context, limit: limit, alignment: alignment, direction: direction)
        InlineLayout(content: content) {
            ForEach(Array(content.children.enumerated()), id: \.element.id) { index, part in
                if let element = part.element {
                    ElementView(element: element, path: part.id)
                        .environment(\.layoutDirection, direction)
                        .padding(.horizontal, 1)
                        .anchorPreference(key: InlineBounds.self, value: .bounds) { [index: $0] }
                }
            }
        }
        .backgroundPreferenceValue(InlineBounds.self) { anchors in
            GeometryReader { geometry in
                let sizes = anchors.mapValues { geometry[$0].size }
                let plan = content.plan(width: geometry.size.width, sizes: sizes)
                ForEach(plan.text, id: \.range.location) { fragment in
                    if let range = Range(fragment.range, in: content.words) {
                        let words = content.words[range]
                        let end = words.characters.last?.isNewline == true ? words.characters.index(before: words.endIndex) : words.endIndex
                        StyledWords(AttributedString(words[..<end]), style: style, preserveLines: nil)
                            .environment(\.layoutDirection, direction)
                            .fixedSize()
                            .frame(width: fragment.rect.width, height: fragment.rect.height, alignment: .leading)
                            .clipContainer()
                            .offset(x: fragment.rect.minX, y: fragment.rect.minY)
                    }
                }
            }
        }
        .clipContainer()
        // Core Text supplies physical positions for both writing directions.
        .environment(\.layoutDirection, .leftToRight)
    }
}

private struct InlineLayout: Layout {
    var content: InlineContent

    struct Cache {
        var content: InlineContent?
        var width: CGFloat?
        var plan: InlineContent.Plan?
        var sizes: [Int: CGSize] = [:]
    }

    func makeCache(subviews: Subviews) -> Cache { Cache() }

    private func measure(_ width: CGFloat?, _ views: Subviews, _ cache: inout Cache) -> InlineContent.Plan {
        if content === cache.content, width == cache.width, let plan = cache.plan { return plan }
        var sizes: [Int: CGSize] = [:]
        for (index, view) in views.enumerated() {
            let ideal = view.sizeThatFits(.unspecified)
            if let width, ideal.width > width {
                sizes[index] = view.sizeThatFits(ProposedViewSize(width: width, height: nil))
            } else {
                sizes[index] = ideal
            }
        }
        let plan = content.plan(width: width, sizes: sizes)
        cache = Cache(content: content, width: width, plan: plan, sizes: sizes)
        return plan
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) -> CGSize {
        measure(proposal.width.flatMap { $0.isFinite ? $0 : nil }, subviews, &cache).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) {
        let plan = measure(bounds.width, subviews, &cache)
        for (i, view) in subviews.enumerated() {
            let rect = plan.children[i] ?? CGRect(origin: CGPoint(x: 0, y: bounds.height), size: cache.sizes[i] ?? .zero)
            view.place(at: CGPoint(x: bounds.minX + rect.minX, y: bounds.minY + rect.minY),
                       anchor: .topLeading, proposal: ProposedViewSize(rect.size))
        }
    }
}
