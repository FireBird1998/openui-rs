import SwiftUI

/// Keep offscreen rows out of the view tree while preserving document size
/// and row identity as the clip moves through it.
struct ViewportRowStack<Row: View>: View {
    let rows: ViewportRows
    let visible: Range<CGFloat>
    let retaining: Int?
    @ViewBuilder var row: (Int, CGFloat) -> Row

    var body: some View {
        let range = rows.visible(in: visible)
        let indices = if let retaining, !range.contains(retaining) {
            retaining < range.lowerBound ? [retaining] + Array(range) : Array(range) + [retaining]
        } else {
            Array(range)
        }
        ZStack(alignment: .topLeading) {
            Color.clear.frame(height: rows.height).accessibilityHidden(true)
            ForEach(indices, id: \.self) { index in
                if let frame = rows.frame(of: index) {
                    row(index, frame.upperBound - frame.lowerBound).offset(y: frame.lowerBound)
                }
            }
        }
        .frame(height: rows.height, alignment: .topLeading)
    }
}
