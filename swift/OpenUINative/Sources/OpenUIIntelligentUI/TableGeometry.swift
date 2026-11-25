import CoreGraphics

/// Give each track an equal share and expand individual tracks to their
/// minimums. A track that overflows does not shrink its neighbors.
struct TableTracks: Equatable {
    let widths: [CGFloat]
    let width: CGFloat

    init(minimums: [CGFloat], available: CGFloat?) {
        let share = (available ?? 0) / CGFloat(max(1, minimums.count))
        widths = minimums.map { max($0, share) }
        width = widths.reduce(0, +)
    }
}

/// Store only row boundaries. A scroll finds the visible slice in logarithmic
/// time and can position it without measuring or mounting preceding rows.
struct ViewportRows: Equatable {
    private let offsets: [CGFloat]
    var height: CGFloat { offsets.last ?? 0 }

    init(heights: some Sequence<CGFloat>, first: CGFloat? = nil) {
        var positions: [CGFloat] = [0]
        var end = first ?? 0
        if first != nil { positions.append(end) }
        for height in heights {
            end += height
            positions.append(end)
        }
        offsets = positions
    }

    func frame(of row: Int) -> Range<CGFloat>? {
        guard row >= 0, row < offsets.count - 1 else { return nil }
        let pair = offsets.dropFirst(row).prefix(2)
        guard let top = pair.first, let bottom = pair.last else { return nil }
        return top..<bottom
    }

    func row(at position: CGFloat) -> Int? {
        guard position >= 0, position < height else { return nil }
        return boundary { $0 > position } - 1
    }

    func visible(in viewport: Range<CGFloat>) -> Range<Int> {
        guard !viewport.isEmpty, viewport.upperBound > 0, viewport.lowerBound < height else { return 0..<0 }
        let first = max(0, boundary { $0 > viewport.lowerBound } - 1)
        let end = min(offsets.count - 1, boundary { $0 >= viewport.upperBound })
        return first..<max(first, end)
    }

    private func boundary(where matches: (CGFloat) -> Bool) -> Int {
        var lower = 0
        var upper = offsets.count
        while lower < upper {
            let middle = lower + (upper - lower) / 2
            if let position = offsets.dropFirst(middle).first, matches(position) {
                upper = middle
            } else {
                lower = middle + 1
            }
        }
        return lower
    }
}
