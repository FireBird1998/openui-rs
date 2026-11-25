import SwiftUI

struct PolarChartLegend: View {
    nonisolated static let sideHeight: CGFloat = 296
    private static let rowHeight: CGFloat = 32
    private static let previewCount = 6

    let data: PolarChartData
    let total: Double
    let beside: Bool
    let hidden: Set<PolarChartData.Label>
    @Binding var showAll: Bool
    @Binding var activeLabel: PolarChartData.Label?
    let toggle: (PolarChartData.Label) -> Void

    @Environment(\.intelligentTheme) private var theme
    @State private var scrollPosition = ScrollPosition()
    @State private var window = ScrollWindow()

    private struct ScrollWindow: Equatable {
        var top: CGFloat = 0
        var maximum: CGFloat = 0
    }

    var body: some View {
        if beside {
            if CGFloat(data.slices.count) * Self.rowHeight > Self.sideHeight {
                VStack(spacing: 0) {
                    scrollHeader
                    ScrollView(.vertical) {
                        LazyVStack(spacing: 0) { rows(data.slices[...]) }
                    }
                    .scrollPosition($scrollPosition)
                    .scrollIndicators(.hidden)
                    .onScrollGeometryChange(for: ScrollWindow.self) { geometry in
                        ScrollWindow(top: max(0, geometry.contentOffset.y),
                                     maximum: max(0, geometry.contentSize.height - geometry.containerSize.height))
                    } action: { _, current in window = current }
                    .clipContainer()
                }
                .frame(height: Self.sideHeight)
            } else {
                VStack(spacing: 0) { rows(data.slices[...]) }
                    .frame(height: Self.sideHeight)
            }
        } else {
            VStack(spacing: 4) {
                LazyVStack(spacing: 0) {
                    rows(showAll ? data.slices[...] : data.slices.prefix(Self.previewCount))
                }
                if data.slices.count > Self.previewCount {
                    Button(showAll ? "Show less" : "Show more") { showAll.toggle() }
                        .frame(maxWidth: .infinity)
                }
            }
        }
    }

    private var scrollHeader: some View {
        HStack(spacing: 6) {
            Text(verbatim: "\(data.slices.count) labels")
                .lineLimit(1)
                .foregroundStyle(theme.muted.color)
            Spacer(minLength: 0)
            scrollButton(up: true).disabled(window.top <= 0.5)
            scrollButton(up: false).disabled(window.top >= window.maximum - 0.5)
        }
        .font(.system(size: 14, design: theme.font))
        .padding(.leading, 18)
        .padding(.trailing, 6)
        .frame(height: Self.rowHeight)
    }

    private func scrollButton(up: Bool) -> some View {
        Button {
            let step = up ? -Self.rowHeight : Self.rowHeight
            scrollPosition.scrollTo(y: max(0, min(window.maximum, window.top + step)))
        } label: {
            if let icon = LucideGlyph.image(named: up ? "chevron-up" : "chevron-down") {
                Image(nsImage: icon).resizable().frame(width: 12, height: 12)
            }
        }
        .accessibilityLabel(up ? "Scroll legend up" : "Scroll legend down")
    }

    private func rows(_ slices: ArraySlice<PolarChartData.Slice>) -> some View {
        ForEach(slices) { slice in
            let isHidden = hidden.contains(slice.label)
            let active = activeLabel == slice.label
            let value = data.legendValue(slice, total: total)
            let label = singleLineLabel(slice.label.text)
            Button {
                toggle(slice.label)
            } label: {
                HStack(spacing: 8) {
                    RoundedRectangle(cornerRadius: 2)
                        .fill(ChartPalette.colour(slice.colourIndex, in: theme))
                        .frame(width: 10, height: 10)
                        .scaleEffect(active ? 1.3 : 1)
                        .frame(width: 32)
                    Text(verbatim: label).lineLimit(1)
                    Spacer(minLength: 2)
                    Text(verbatim: value).lineLimit(1).fixedSize()
                }
                .padding(.horizontal, 8)
                .frame(maxWidth: .infinity)
                .frame(height: Self.rowHeight)
                .font(.system(size: 14, design: theme.font))
                .foregroundStyle(theme.ink.color)
                .background(active ? theme.surface.color : .clear, in: RoundedRectangle(cornerRadius: theme.radiusMd))
                .opacity(isHidden ? 0.3 : 1)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text(verbatim: (isHidden ? "Show " : "Hide ") + label + ", " + value))
            .onHover { over in
                if over {
                    activeLabel = slice.label
                } else if activeLabel == slice.label {
                    activeLabel = nil
                }
            }
        }
    }

    private func singleLineLabel(_ text: String) -> String {
        // CSS nowrap collapses spaces, tabs and line breaks, preserving
        // nonbreaking spaces and the original category key.
        text.unicodeScalars.split { scalar in
            scalar == " " || scalar == "\t" || scalar == "\n" || scalar == "\r"
        }.map(String.init).joined(separator: " ")
    }
}
