import OpenUINative
import SwiftUI

private struct CarouselItem: Identifiable {
    let id: String
    let children: [LayoutChild]

    // The lib wrapper flattens one list level before removing nulls. Two
    // adjacent strings are two items, and a deeper list stays in one item.
    static func items(_ value: PropValue?) -> [Self] {
        guard case let .list(values) = value else { return [] }
        var result: [Self] = []
        func append(_ value: PropValue, at path: String) {
            if case .scalar(.null) = value { return }
            result.append(Self(id: path, children: Element.layoutChildren(value, at: path)))
        }
        for (i, value) in values.enumerated() {
            let path = "children/\(i)"
            if case let .list(list) = value {
                for (j, child) in list.enumerated() { append(child, at: "\(path)/\(j)") }
            } else {
                append(value, at: path)
            }
        }
        return result
    }
}

// Collect laid-out descendant bounds in the track's coordinates. Snap
// targets retain their capped widths even when nested layout extends farther.
struct CarouselBounds: PreferenceKey {
    static var defaultValue: [Anchor<CGRect>] { [] }
    static func reduce(value: inout [Anchor<CGRect>], nextValue: () -> [Anchor<CGRect>]) {
        value.append(contentsOf: nextValue())
    }
}

private struct CarouselViewport: Equatable {
    var width: CGFloat = 0
    var content: CGFloat = 0
    var offset: CGFloat = 0
    var previous: Bool { offset > 0 }
    var next: Bool { ceil(offset) + width < content }
}

struct LayoutCarousel: View {
    private let items: [CarouselItem]
    private let itemWidth: CGFloat
    private let gap: CGFloat
    private let path: String
    @State private var contentExtent: CGFloat = 0
    @State private var viewport = CarouselViewport()
    @Environment(\.reducesMotion) private var reduceMotion
    @Environment(\.displayScale) private var displayScale
    @State private var position = ScrollPosition(x: 0)

    init(element: Element, path: String) {
        self.path = path
        items = CarouselItem.items(element.prop("children"))
        itemWidth = min(640, max(80, element.prop("itemWidth")?.ruleScalar?.number ?? 240))
        gap = layoutPoints(Spacing.points(element.prop("gap")) ?? 12)
    }

    var body: some View {
        ScrollView(.horizontal) {
            HStack(alignment: .top, spacing: gap) {
                ForEach(items) { item in
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(item.children) { child in
                            let type = child.element?.typeName
                            child.drawCell(in: path, stretches: type == "Card" || type == "Pressable")
                        }
                    }
                    .containerRelativeFrame(.horizontal, alignment: .leading) { width, _ in
                        usedItemWidth(in: width)
                    }
                }
            }
            .scrollTargetLayout()
            .backgroundPreferenceValue(CarouselBounds.self) { bounds in
                GeometryReader { track in
                    let end = bounds.reduce(CGFloat.zero) { max($0, track[$1].maxX) }
                    Color.clear.onChange(of: layoutPoints(end), initial: true) { _, width in contentExtent = width }
                }
            }
            .frame(minWidth: contentExtent, alignment: .leading)
            .frame(minHeight: 0, maxHeight: .infinity, alignment: .topLeading)
        }
        .scrollIndicators(.hidden)
        .scrollTargetBehavior(.viewAligned(anchor: .leading))
        .scrollPosition($position)
        .onScrollGeometryChange(for: CarouselViewport.self) { geometry in
            CarouselViewport(width: geometry.containerSize.width, content: geometry.contentSize.width,
                             offset: geometry.contentOffset.x)
        } action: { _, new in viewport = new }
        .frame(maxWidth: naturalTrackWidth, alignment: .leading)
        .clipContainer()
        .mask {
            HStack(spacing: 0) {
                if viewport.previous { fade(from: .leading, to: .trailing) }
                Color.black
                if viewport.next { fade(from: .trailing, to: .leading) }
            }
        }
        .frame(minWidth: 0, idealWidth: naturalTrackWidth, maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .leading) { if viewport.previous { arrow(previous: true) } }
        .overlay(alignment: .trailing) { if viewport.next { arrow(previous: false) } }
        .clipContainer()
    }

    private var naturalTrackWidth: CGFloat {
        layoutPoints(CGFloat(items.count) * itemWidth + CGFloat(max(0, items.count - 1)) * gap)
    }

    private func usedItemWidth(in viewportWidth: CGFloat) -> CGFloat {
        min(itemWidth, 0.85 * viewportWidth)
    }

    private func fade(from: UnitPoint, to: UnitPoint) -> some View {
        LinearGradient(colors: [.clear, .black], startPoint: from, endPoint: to)
            .frame(width: min(40, viewport.width / 2))
    }

    private func arrow(previous: Bool) -> some View {
        CarouselArrow(previous: previous) {
            let stride = usedItemWidth(in: viewport.width) + gap
            guard stride > 0 else { return }
            // The scroll view rounds a settled item edge to a device pixel.
            // Between edges, upstream starts at the first fully visible item.
            let index = viewport.offset / stride
            let nearest = index.rounded()
            let firstVisible = abs(viewport.offset - nearest * stride) <= 1 / displayScale ? nearest : ceil(index)
            // Overflow can scroll every item origin past the leading edge.
            let last = CGFloat(max(0, items.count - 1))
            let current = min(last, max(0, firstVisible))
            let destination = min(last, max(0, current + (previous ? -1 : 1)))
            let target = min(max(0, viewport.content - viewport.width), destination * stride)
            withAnimation(reduceMotion ? nil : .easeOut(duration: 0.22)) { position.scrollTo(x: target) }
        }
        .padding(.horizontal, 8)
    }
}

private struct CarouselArrow: View {
    let previous: Bool
    let action: () -> Void
    @State private var hover = false
    @Environment(\.intelligentTheme) private var theme

    var body: some View {
        Button(action: action) {
            Image(systemName: previous ? "chevron.left" : "chevron.right")
                .font(.system(size: 16)).foregroundStyle(theme.ink.color)
                .frame(width: 34, height: 34)
                .background((hover ? theme.glassHover : theme.glass).color, in: Circle())
                .background(.thinMaterial, in: Circle())
                .overlay { Circle().strokeBorder((hover ? theme.glassRingHover : theme.glassRing).color) }
                // The overlay proposes the track's size. Give the button itself
                // only the visible slice, even when AppKit targets it directly.
                .frame(minWidth: 0, maxWidth: 34, minHeight: 0, maxHeight: 34)
                .clipContainer(to: ArrowHitShape())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .accessibilityLabel(previous ? "Previous" : "Next")
    }
}

// Preserve the 34-point circle when the track crops its top or sides.
private struct ArrowHitShape: Shape {
    func path(in rect: CGRect) -> Path {
        let circle = CGRect(x: rect.midX - 17, y: rect.midY - 17, width: 34, height: 34)
        return Path(ellipseIn: circle).intersection(Path(rect))
    }
}
