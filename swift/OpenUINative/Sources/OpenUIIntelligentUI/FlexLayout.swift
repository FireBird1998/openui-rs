// The six layout components share one flex formatting context. Measure a
// line's bases, distribute its remaining space, then measure wrapped text at
// its allocated width. Placement uses those same measurements.
import SwiftUI

// WebKit stores layout positions in a signed 26.6 fixed-point LayoutUnit.
// CSS length conversion has a slightly smaller positive bound.
private let layoutMin = CGFloat(Int32.min) / 64
private let layoutMax = CGFloat(Int32.max) / 64
private let cssLengthMax: CGFloat = 33_554_428

private func layoutPosition(_ n: CGFloat) -> CGFloat { min(layoutMax, max(layoutMin, n)) }
func layoutPoints(_ n: CGFloat) -> CGFloat { min(layoutMax, max(0, n)) }

extension Length {
    // An unresolved percentage still has a computed percentage size. Only
    // auto (including an omitted or invalid grid length) can stretch.
    var isAuto: Bool {
        switch self {
        case .auto, .fr: true
        default: false
        }
    }

    func isDefinite(in definiteParent: Bool) -> Bool {
        switch self {
        case .percent: definiteParent
        case .auto, .fr: false
        default: true
        }
    }

    func points(relativeTo parent: CGFloat?, viewport: CGSize?, em: CGFloat) -> CGFloat? {
        let value: CGFloat?
        switch self {
        case let .points(n): value = n
        case let .percent(n): value = parent.map { $0 * (n / 100) }
        case let .vw(n): value = viewport.map { $0.width * (n / 100) }
        case let .vh(n): value = viewport.map { $0.height * (n / 100) }
        case let .em(n): value = em * n
        case let .rem(n): value = 16 * n
        case .auto, .fr: value = nil // fr is invalid outside a grid track.
        }
        return value.map { min(cssLengthMax, max(0, $0)) }
    }
}

enum FlexAlign: String { case start, center, end, stretch }
enum FlexJustify: String { case start, center, end, between }

struct FlexItem: LayoutValueKey {
    static let defaultValue = FlexItem()
    var width: Length?
    var height: Length?
    var grow: Double?
    var wrapsColumn = false
    var fixed = false
    var photo = false
    var photoBox = false
    var boxedPhoto = false
    var centered = false
    var margin: CGFloat = 0
    var minimum: CGFloat = 0

    func stretches(inRow row: Bool, align: FlexAlign) -> Bool {
        let length = row ? height : width
        return align == .stretch && !(row && centered) && (length?.isAuto ?? true)
    }
}

// Ask for the height occupied by content independently of the border box
// a proposal assigns. A wrapping column can report this after line breaking.
private enum FlexContentHeight: AlignmentID {
    static func defaultValue(in dimensions: ViewDimensions) -> CGFloat { dimensions.height }
}

extension VerticalAlignment {
    static let flexContentHeight = VerticalAlignment(FlexContentHeight.self)
}

struct FlexLayout: Layout {
    var row: Bool
    var align: FlexAlign
    var justify: FlexJustify
    var wrap: Bool
    var gap: CGFloat
    var inset: CGFloat
    var viewport: CGSize?
    var em: CGFloat
    var acceptsHeight: Bool
    var definiteHeight: Bool
    var narrowInset: CGFloat?
    var ownWidth: Length?
    var ownHeight: Length?

    private func size(main: CGFloat, cross: CGFloat) -> CGSize {
        row ? CGSize(width: main, height: cross) : CGSize(width: cross, height: main)
    }

    private func proposal(main: CGFloat?, cross: CGFloat?) -> ProposedViewSize {
        row ? ProposedViewSize(width: main, height: cross) : ProposedViewSize(width: cross, height: main)
    }

    private struct Item {
        let view: LayoutSubview
        let basis: CGFloat
        let minimum: CGFloat
        var hypothetical: CGFloat { max(basis, minimum) }
        let shrinkWeight: CGFloat
        let cross: CGFloat?
        let grow: CGFloat
        let margin: CGFloat
        let stretches: Bool
        let align: FlexAlign
    }

    private struct Line {
        var items: [Item] = []
        var used: CGFloat = 0
        var onlyPercentageWidths = true

        func mainSizes(in available: CGFloat?) -> [CGFloat] {
            let delta = (available ?? used) - used
            if delta == 0 { return items.map(\.hypothetical) }
            if delta < 0 {
                // Scale shrink by the inner flex basis. Once a child reaches
                // its minimum, redistribute its remaining deficit to siblings.
                let candidates = items.filter { $0.shrinkWeight > 0 && $0.basis > $0.minimum }
                var weight = candidates.reduce(0) { $0 + $1.shrinkWeight }
                var deficit = -delta
                let constrained = candidates.sorted {
                    ($0.basis - $0.minimum) / $0.shrinkWeight < ($1.basis - $1.minimum) / $1.shrinkWeight
                }
                for item in constrained {
                    let capacity = item.basis - item.minimum
                    guard deficit * item.shrinkWeight > capacity * weight else { break }
                    deficit -= capacity
                    weight -= item.shrinkWeight
                }
                let fraction = weight > 0 ? deficit / weight : 1
                return items.map { max($0.minimum, $0.basis - $0.shrinkWeight * fraction) }
            }
            // A zero flex basis includes the border and padding. Freeze
            // items at that minimum if their proportional share is smaller.
            let scale = max(1, items.lazy.map(\.grow).max() ?? 0)
            var total = items.reduce(0) { $0 + $1.grow / scale }
            guard total > 0 else { return items.map(\.hypothetical) }
            let bounded = items.filter { $0.grow > 0 && $0.minimum > $0.basis }
            let free = delta + bounded.reduce(0) { $0 + $1.minimum - $1.basis }
            var remaining = free
            func share(_ item: Item) -> CGFloat {
                guard total > 0, item.grow > 0 else { return 0 }
                return min(free * item.grow, remaining * min(1, (item.grow / scale) / total))
            }
            // The largest minimum per unit of growth freezes first. One
            // bounded pass suffices, even when each minimum is different.
            let constrained = bounded.sorted {
                ($0.minimum - $0.basis) / $0.grow > ($1.minimum - $1.basis) / $1.grow
            }
            for item in constrained {
                let required = item.minimum - item.basis
                guard share(item) < required else { break }
                remaining = max(0, remaining - required)
                total -= item.grow / scale
            }
            return items.map { max($0.minimum, $0.basis + share($0)) }
        }
    }

    private typealias MeasuredLine = (items: [(Item, CGFloat, CGFloat)], main: CGFloat, cross: CGFloat)

    private func lines(_ views: Subviews, width: CGFloat?, height: CGFloat?, viewport port: CGSize) -> [Line] {
        let main = row ? width : height, cross = row ? height : width
        var lines: [Line] = []
        var line = Line()
        for view in views {
            let item = view[FlexItem.self]
            let media = row && (item.photo || item.photoBox)
            let w = media ? min(200, max(120, (width ?? 0) * 0.33))
                : item.width?.points(relativeTo: width, viewport: port, em: em)
            let photoWidth = media && item.photo ? w : item.boxedPhoto ? width : nil
            let h = photoWidth.map { $0 * 0.75 } ?? item.height?.points(relativeTo: definiteHeight ? height : nil, viewport: port, em: em)
            let alignment = row && item.centered ? .center : align
            let stretches = item.stretches(inRow: row, align: align)
            var explicitCross = (row ? h : w).map { max(item.minimum, $0) }
            // An aligned column child is shrink-to-fit. Measure its height
            // with that width so a centered paragraph can wrap.
            if !row, !wrap, alignment != .stretch, explicitCross == nil, let width {
                explicitCross = min(width, view.sizeThatFits(.unspecified).width)
            }
            let measureCross = explicitCross ?? (stretches && !wrap ? cross : nil)
            // Measure the automatic min-height at the used width, independent
            // of this child's proposed flex height. Row children have min-width: 0.
            lazy var ideal = view.sizeThatFits(proposal(main: nil, cross: measureCross))
            var minimum = item.minimum
            if !row, main != nil, definiteHeight {
                // A specified height determines where a wrapping column breaks
                // its lines. Other items measure their content at ideal height.
                let query = proposal(main: item.wrapsColumn ? h : nil, cross: measureCross)
                let content = view.dimensions(in: query)[VerticalAlignment.flexContentHeight]
                minimum = max(minimum, min(layoutPoints(content), h ?? layoutMax))
            }
            let basis: CGFloat
            if item.grow != nil, main != nil, row || definiteHeight, !media {
                basis = 0
            } else if !row, item.grow != nil {
                // flex-basis: 0% resolves to content at the used width when
                // the column height is indefinite, even with a preferred height.
                basis = layoutPoints(ideal.height)
            } else if let length = row ? w : h {
                basis = length
            } else {
                basis = layoutPoints(row ? ideal.width : ideal.height)
            }
            let needed = max(minimum, basis) + item.margin
            if wrap, let main {
                // Rounded percentages can overfill a row by up to one point.
                // Count gaps and margins in the fit and flex the actual sizes below.
                let percentageWidth = if case .percent = item.width {
                    row && !media && item.grow == nil && basis >= minimum
                } else { false }
                let tolerance: CGFloat = line.onlyPercentageWidths && percentageWidth ? 1 : 0
                if !line.items.isEmpty, line.used + gap + needed > main + tolerance {
                    lines.append(line)
                    line = Line()
                }
                line.onlyPercentageWidths = line.onlyPercentageWidths && percentageWidth
            }
            if !line.items.isEmpty { line.used += gap }
            line.used += needed
            let shrinkWeight = media || item.fixed ? 0 : max(0, basis - item.minimum)
            line.items.append(Item(view: view, basis: basis, minimum: minimum, shrinkWeight: shrinkWeight,
                                   cross: explicitCross, grow: media ? 0 : item.grow ?? 0,
                                   margin: item.margin, stretches: stretches, align: alignment))
        }
        if !line.items.isEmpty { lines.append(line) }
        return lines
    }

    private func measure(_ line: Line, main: CGFloat?, cross: CGFloat?) -> MeasuredLine {
        var used = line.used, breadth: CGFloat = 0
        var measured: [(Item, CGFloat, CGFloat)] = []
        for (item, allocated) in zip(line.items, line.mainSizes(in: main)) {
            let measuredCross: CGFloat
            if let definite = item.cross {
                measuredCross = definite
            } else {
                let stretch = item.stretches && !wrap
                let ideal = item.view.sizeThatFits(proposal(main: allocated, cross: stretch ? cross : nil))
                let natural = layoutPoints(row ? ideal.height : ideal.width)
                measuredCross = max(item.view[FlexItem.self].minimum, natural)
            }
            used += allocated - item.hypothetical
            breadth = max(breadth, measuredCross)
            measured.append((item, allocated, measuredCross))
        }
        return (measured, used, breadth)
    }

    private func plan(_ proposed: ProposedViewSize, _ views: Subviews) -> (size: CGSize, placements: [(LayoutSubview, CGRect)], contentHeight: CGFloat) {
        let port = viewport ?? proposed.replacingUnspecifiedDimensions(by: .zero)
        let compact = port.width > 0 && port.width <= 640
        let padding = layoutPoints(compact ? narrowInset ?? inset : inset)
        let proposed = ProposedViewSize(
            width: ownWidth?.points(relativeTo: proposed.width, viewport: port, em: em) ?? proposed.width,
            height: ownHeight?.points(relativeTo: proposed.height, viewport: port, em: em)
                ?? (acceptsHeight ? proposed.height : nil)
        )
        let width = proposed.width.flatMap { $0.isFinite ? layoutPoints($0 - 2 * padding) : nil }
        let height = proposed.height.flatMap { $0.isFinite ? layoutPoints($0 - 2 * padding) : nil }
        let main = row ? width : height, cross = row ? height : width
        let measured = lines(views, width: width, height: height, viewport: port).map {
            measure($0, main: main, cross: cross)
        }

        let intrinsicCross = measured.reduce(0) { $0 + $1.cross } + CGFloat(max(0, measured.count - 1)) * gap
        let extraCross = max(0, (cross ?? intrinsicCross) - intrinsicCross) / CGFloat(max(1, measured.count))
        var crossOrigin = padding
        var placements: [(LayoutSubview, CGRect)] = []
        for line in measured {
            let breadth = !wrap ? cross ?? line.cross : line.cross + extraCross
            let spare = layoutPosition((main ?? line.main) - line.main)
            let between = justify == .between && line.items.count > 1 ? max(0, spare) / CGFloat(line.items.count - 1) : 0
            var cursor = layoutPosition(padding + (justify == .end ? spare : justify == .center ? spare / 2 : 0))
            for (item, extent, natural) in line.items {
                cursor = layoutPosition(cursor + item.margin)
                let span = item.stretches && item.cross == nil ? breadth : natural
                let offset = item.align == .end ? breadth - span : item.align == .center ? (breadth - span) / 2 : 0
                let origin = size(main: cursor, cross: layoutPosition(crossOrigin + offset))
                placements.append((item.view, CGRect(origin: CGPoint(x: origin.width, y: origin.height),
                                                     size: size(main: extent, cross: span))))
                cursor = layoutPosition(cursor + extent + gap + between)
            }
            crossOrigin = layoutPosition(crossOrigin + breadth + gap)
        }
        let intrinsicMain = measured.map(\.main).max() ?? 0
        let result = size(main: main ?? intrinsicMain, cross: cross ?? intrinsicCross)
        return (CGSize(width: layoutPoints(result.width + 2 * padding), height: layoutPoints(result.height + 2 * padding)),
                placements, layoutPoints((row ? intrinsicCross : intrinsicMain) + 2 * padding))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        plan(proposal, subviews).size
    }

    func explicitAlignment(of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize,
                           subviews: Subviews, cache: inout ()) -> CGFloat? {
        guard guide == .flexContentHeight else { return nil }
        return wrap && !row ? plan(proposal, subviews).contentHeight : bounds.height
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (view, rect) in plan(proposal, subviews).placements {
            view.place(at: CGPoint(x: bounds.minX + rect.minX, y: bounds.minY + rect.minY),
                       anchor: .topLeading, proposal: ProposedViewSize(rect.size))
        }
    }
}
