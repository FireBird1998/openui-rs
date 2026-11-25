import OpenUINative
import SwiftUI

struct IntelligentPhoto: View {
    let element: Element
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.usesMediaPhotoRatio) private var usesMediaPhotoRatio
    @Environment(\.liveProgram) private var live
    @State private var failedSource: String?

    var body: some View {
        let value = element.prop("aspectRatio")?.ruleScalar?.number ?? 1.5
        let ratio = usesMediaPhotoRatio ? mediaPhotoRatio : (value > 0.2 && value < 5 ? value : 1.5)
        let radiusProp = element.prop("radius")
        let radius = switch radiusProp {
        case nil, .scalar(.null)?: theme.radiusLg
        default: Radius(radiusProp).map(theme.radius) ?? 0
        }
        let bounds = PhotoBounds(ratio: ratio, radius: layoutPoints(radius))
        let alt = JSWhitespace.trim(element.prop("alt")?.ruleScalar?.string ?? "")
        let address = JSWhitespace.trim(element.prop("src")?.ruleScalar?.string ?? "")
        let fit: ContentMode = element.prop("fit")?.ruleScalar?.string == "contain" ? .fit : .fill
        let settled = !element.partial && live?.hasUnfinishedStatements != true
        bounds.fill(theme.photoEmpty.color)
            .overlay {
                if settled, address != failedSource, let source = PhotoSource(address) {
                    PhotoContent(source: source, fit: fit, alt: alt) { failedSource = address }
                        .id(address)
                } else {
                    EmptyPhoto()
                        .accessibilityElement(children: .ignore)
                        .accessibilityAddTraits(.isImage)
                        .accessibilityLabel(Text(verbatim: alt))
                }
            }
            .clipContainer(to: bounds)
    }
}

private struct PhotoContent: View {
    let source: PhotoSource
    let fit: ContentMode
    let alt: String
    let onFailure: () -> Void
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var placement: PhotoPlacement?
    @State private var intersectsScrollViewport = false
    @State private var loaded: Loaded?
    @State private var opacity = 0.0

    private struct Loaded {
        let raster: PhotoRaster
        let start: Date
        let reservation: PhotoBudget.Reservation
    }

    var body: some View {
        let admission = intersectsScrollViewport ? placement : nil
        GeometryReader { proxy in
            Group {
                if let loaded {
                    bitmap(loaded).opacity(opacity)
                } else if admission != nil {
                    shimmer
                }
            }
            .frame(width: proxy.size.width, height: proxy.size.height)
        }
        .background {
            PhotoViewport(cover: fit == .fill) { placement = $0 }
                .accessibilityHidden(true)
        }
        .onGeometryChange(for: Bool.self, of: Self.intersectsScrollViewport) { visible in
            intersectsScrollViewport = visible
        }
        .onChange(of: admission) {
            loaded = nil
            opacity = 0
        }
        .task(id: admission) {
            guard let admission, let reservation = await admission.budget.reserve(),
                  !Task.isCancelled, placement == admission, intersectsScrollViewport else { return }
            let raster = await PhotoLoader.load(source, size: admission.size)
            guard !Task.isCancelled, placement == admission, intersectsScrollViewport else { return }
            if let raster {
                reservation.retain(raster)
                loaded = Loaded(raster: raster, start: Date(), reservation: reservation)
                withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.3)) { opacity = 1 }
            } else {
                onFailure()
            }
        }
        .onDisappear {
            loaded = nil
            opacity = 0
        }
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isImage)
        .accessibilityLabel(Text(verbatim: alt))
        .accessibilityHidden(alt.isEmpty)
    }

    @ViewBuilder private func bitmap(_ loaded: Loaded) -> some View {
        if let animation = loaded.raster.animation {
            TimelineView(.animation(minimumInterval: 1.0 / 60)) { context in
                let frame = animation.frame(at: context.date.timeIntervalSince(loaded.start))
                bitmap(frame ?? loaded.raster.first)
            }
        } else {
            bitmap(loaded.raster.first)
        }
    }

    private func bitmap(_ image: CGImage) -> some View {
        Image(decorative: image, scale: 1).resizable().aspectRatio(contentMode: fit)
    }

    private var shimmer: some View {
        TimelineView(.animation(paused: reduceMotion)) { context in
            let cycle = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1.4) / 1.4
            let center = reduceMotion ? 0.5 : 1.4 - 1.8 * cycle
            LinearGradient(colors: [.clear, theme.canvas.color.opacity(0.55), .clear],
                           startPoint: UnitPoint(x: center - 0.35, y: 0.4),
                           endPoint: UnitPoint(x: center + 0.35, y: 0.6))
        }
    }

    nonisolated private static func intersectsScrollViewport(_ proxy: GeometryProxy) -> Bool {
        // SwiftUI also clips scrolling content in its own coordinate spaces.
        for axis in [Axis.horizontal, .vertical] {
            let space = NamedCoordinateSpace.scrollView(axis: axis)
            guard let viewport = proxy.bounds(of: space) else { continue }
            let visible = CGRect(origin: .zero, size: viewport.size).intersection(proxy.frame(in: space))
            if visible.isEmpty { return false }
        }
        return true
    }
}

private struct EmptyPhoto: View {
    @Environment(\.intelligentTheme) private var theme

    var body: some View {
        if let glyph = LucideGlyph.image(named: "image", strokeWidth: 1.5) {
            Image(nsImage: glyph).resizable()
                .foregroundStyle(theme.photoEmptyIcon.color)
                .frame(width: 26, height: 26)
        }
    }
}

private struct PhotoBounds: Shape {
    let ratio: CGFloat
    let radius: CGFloat

    // Use the proposed width as CSS does and derive height from the ratio.
    func sizeThatFits(_ proposal: ProposedViewSize) -> CGSize {
        let width = layoutPoints(proposal.width ?? 240)
        return CGSize(width: width, height: layoutPoints(width / ratio))
    }

    func path(in rect: CGRect) -> Path {
        Path(roundedRect: rect, cornerRadius: min(radius, min(rect.width, rect.height) / 2))
    }
}
