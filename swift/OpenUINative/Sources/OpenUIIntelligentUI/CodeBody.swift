import AppKit
import CoreText
import SwiftUI

/// A source index, with one byte offset per row and horizontal checkpoints
/// only on long rows. No text views or shaped glyphs are kept for the document.
@MainActor private struct CodeDocument {
    struct Segment {
        let bytes: Range<Int>
        let x: CGFloat
    }

    static let font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
    static let advance = (" " as NSString).size(withAttributes: [.font: font]).width
    private static let paragraphStyle: NSParagraphStyle = {
        let style = NSMutableParagraphStyle()
        style.baseWritingDirection = .leftToRight
        return style
    }()
    private static let segmentBytes = 256
    static let lineHeight: CGFloat = 20.8
    static let inset = CGPoint(x: 16, y: 14)

    let source: String
    private(set) var width: CGFloat = 0
    private var starts = [0]
    private var longRows: [Int: [Segment]] = [:]

    var rowCount: Int { starts.count - 1 }
    var size: CGSize {
        CGSize(width: width + 2 * Self.inset.x,
               height: CGFloat(rowCount) * Self.lineHeight + 2 * Self.inset.y)
    }

    init(_ source: String) {
        var source = source
        source.makeContiguousUTF8()
        self.source = source
        guard !source.isEmpty else { return }
        source.withUTF8 { bytes in
            var rowStart = 0
            var start = 0
            var offset = 0
            var columns = 0
            var needsMetrics = false
            var hasTab = false
            var widths: [Data: CGFloat] = [:]
            var x: CGFloat = 0
            var segments: [Segment] = []

            @MainActor func finishSegment(record: Bool) {
                let segment = Segment(bytes: start..<offset, x: x)
                let end: CGFloat
                if !needsMetrics {
                    end = x + CGFloat(columns) * Self.advance
                } else if hasTab && x != 0 {
                    end = Self.measure(bytes, segment: segment)
                } else {
                    // Fallback fonts have variable advances. Measure each small
                    // slice without retaining glyphs, and reuse repeated slices.
                    let key = Data(bytes.dropFirst(start).prefix(segment.bytes.count))
                    if let width = widths[key] {
                        end = x + width
                    } else {
                        end = Self.measure(bytes, segment: segment)
                        if widths.count < 512 { widths[key] = end - x }
                    }
                }
                // Collapse coincident checkpoints, even on a pathological line
                // made entirely of zero-advance controls or combining marks.
                if record && (end > x || segments.last?.x != x) { segments.append(segment) }
                x = end
                start = offset
                columns = 0
                needsMetrics = false
                hasTab = false
            }

            @MainActor func finishRow() {
                finishSegment(record: !segments.isEmpty)
                if !segments.isEmpty { longRows[starts.count - 1] = segments }
                width = max(width, x)
                rowStart = offset + 1
                starts.append(rowStart)
                start = rowStart
                x = 0
                segments.removeAll(keepingCapacity: true)
            }

            for byte in bytes {
                if byte == 10 {
                    finishRow()
                } else {
                    // Split at a UTF-8 scalar boundary. Even an exceptionally
                    // long cluster can't make a Core Text run grow unbounded.
                    if offset - start >= Self.segmentBytes && byte & 0xc0 != 0x80 {
                        finishSegment(record: true)
                    }
                    switch byte {
                    case 9:
                        hasTab = true
                        let column = Int((x / Self.advance + 0.000001).rounded(.down)) + columns
                        columns += 8 - column % 8
                    case 13: break
                    case 32...126: columns += 1
                    default: needsMetrics = true
                    }
                }
                offset += 1
            }
            // A tail containing only bare CRs has no visible row.
            if bytes.dropFirst(rowStart).contains(where: { $0 != 13 }) {
                finishRow()
            }
        }
    }

    func row(_ index: Int) -> Range<Int>? {
        guard let lower = starts.dropFirst(index).first,
              let next = starts.dropFirst(index + 1).first else { return nil }
        let upper = next - 1
        return lower..<upper
    }

    func segments(row index: Int, visible: Range<CGFloat>) -> ArraySlice<Segment> {
        guard let segments = longRows[index] else {
            guard let range = row(index), !range.isEmpty else { return [] }
            return [Segment(bytes: range, x: 0)][...]
        }
        // Seek directly to the segment crossing the left clip edge.
        var low = 0
        var high = segments.count
        while low < high {
            let mid = low + (high - low) / 2
            guard let segment = segments.dropFirst(mid).first else { break }
            if segment.x < visible.lowerBound { low = mid + 1 } else { high = mid }
        }
        return segments.dropFirst(max(0, low - 1))
    }

    func matches(_ candidate: String) -> Bool {
        var current = source
        var candidate = candidate
        return current.withUTF8 { lhs in
            candidate.withUTF8 { rhs in
                lhs.count == rhs.count && (lhs.baseAddress == rhs.baseAddress || lhs.elementsEqual(rhs))
            }
        }
    }

    func runs(_ segment: Segment) -> [Run] {
        var source = source
        return source.withUTF8 { Self.runs($0, segment: segment).runs }
    }

    struct Run {
        let bytes: Range<Int>
        let x: CGFloat
        let end: CGFloat
        let text: String
        let line: CTLine?

        func x(at byte: Int) -> CGFloat {
            guard let line else { return byte <= bytes.lowerBound ? x : end }
            let utf16 = String(decoding: text.utf8.prefix(byte - bytes.lowerBound), as: UTF8.self).utf16.count
            return x + CTLineGetOffsetForStringIndex(line, utf16, nil)
        }

        func byte(at x: CGFloat) -> Int {
            guard let line else { return x < (self.x + end) / 2 ? bytes.lowerBound : bytes.upperBound }
            let offset = CTLineGetStringIndexForPosition(line, CGPoint(x: x - self.x, y: 0))
            guard offset >= 0,
                  let unit = text.utf16.index(text.utf16.startIndex, offsetBy: offset,
                                               limitedBy: text.utf16.endIndex),
                  let index = unit.samePosition(in: text.utf8) else { return bytes.lowerBound }
            return bytes.lowerBound + text.utf8.distance(from: text.utf8.startIndex, to: index)
        }
    }

    private static func measure(_ bytes: UnsafeBufferPointer<UInt8>, segment: Segment) -> CGFloat {
        autoreleasepool { runs(bytes, segment: segment).end }
    }

    private static func runs(_ bytes: UnsafeBufferPointer<UInt8>, segment: Segment) -> (runs: [Run], end: CGFloat) {
        var runs: [Run] = []
        var x = segment.x
        var start = segment.bytes.lowerBound

        func appendText(until end: Int) {
            guard start < end else { return }
            let text = String(decoding: bytes.dropFirst(start).prefix(end - start), as: UTF8.self)
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: [
                .font: font, .ligature: 0, .paragraphStyle: paragraphStyle,
                NSAttributedString.Key(kCTForegroundColorFromContextAttributeName as String): true,
            ]))
            let endX = x + CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
            runs.append(Run(bytes: start..<end, x: x, end: endX, text: text, line: line))
            x = endX
        }

        for (i, byte) in bytes.dropFirst(start).prefix(segment.bytes.count).enumerated() where byte == 9 || byte == 13 {
            let offset = segment.bytes.lowerBound + i
            appendText(until: offset)
            if byte == 9 {
                let stop = 8 * advance
                let end = ((x / stop + 0.000001).rounded(.down) + 1) * stop
                runs.append(Run(bytes: offset..<(offset + 1), x: x, end: end, text: "", line: nil))
                x = end
            }
            start = offset + 1
        }
        appendText(until: segment.bytes.upperBound)
        return (runs, x)
    }
}

struct CodeBody: NSViewRepresentable {
    let source: String
    let ink: ThemeColor
    let write: @MainActor (String) -> Bool

    func makeNSView(context: Context) -> CodeScrollView {
        CodeScrollView(source: source, ink: ink, write: write)
    }

    func updateNSView(_ scroll: CodeScrollView, context: Context) {
        scroll.update(source: source, ink: ink, write: write)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: CodeScrollView, context: Context) -> CGSize? {
        let size = nsView.documentSize
        let width = proposal.width.flatMap { $0.isFinite ? $0 : nil } ?? size.width
        return CGSize(width: width, height: size.height)
    }
}

/// The scroll document carries the full dimensions. Its only painting subview
/// tracks the onscreen rectangle, so neither AppKit nor Core Animation needs
/// a backing surface as wide or as tall as the source.
final class CodeScrollView: NSScrollView {
    private let document = NSView()
    private let surface: CodeSurface

    var documentSize: CGSize { surface.code.size }

    init(source: String, ink: ThemeColor, write: @escaping @MainActor (String) -> Bool) {
        surface = CodeSurface(source: source, ink: ink, write: write)
        super.init(frame: .zero)
        drawsBackground = false
        hasHorizontalScroller = true
        scrollerStyle = .overlay
        horizontalScrollElasticity = .none
        verticalScrollElasticity = .none
        automaticallyAdjustsContentInsets = false
        documentView = document
        document.addSubview(surface)
        document.setAccessibilityElement(false)
        contentView.postsBoundsChangedNotifications = true
        resizeDocument()
    }

    required init?(coder: NSCoder) { return nil }

    func update(source: String, ink: ThemeColor, write: @escaping @MainActor (String) -> Bool) {
        if !surface.code.matches(source) {
            surface.code = CodeDocument(source)
            resizeDocument()
        }
        if surface.ink != ink { surface.ink = ink }
        surface.write = write
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        let center = NotificationCenter.default
        center.removeObserver(self, name: NSView.boundsDidChangeNotification, object: nil)
        guard window != nil else { return }
        var ancestor: NSView? = contentView
        while let view = ancestor {
            if let clip = view as? NSClipView {
                clip.postsBoundsChangedNotifications = true
                center.addObserver(self, selector: #selector(clipMoved),
                                   name: NSView.boundsDidChangeNotification, object: clip)
            }
            ancestor = view.superview
        }
        updateSurface()
    }

    @objc private func clipMoved() { updateSurface() }

    override func layout() {
        super.layout()
        updateSurface()
    }

    override func viewWillDraw() {
        updateSurface()
        super.viewWillDraw()
    }

    private func resizeDocument() {
        document.setFrameSize(documentSize)
        contentView.scroll(to: contentView.constrainBoundsRect(contentView.bounds).origin)
        reflectScrolledClipView(contentView)
        invalidateIntrinsicContentSize()
        updateSurface()
    }

    private func updateSurface() {
        let visible = document.visibleRect
        // Offscreen hosting views also have a visibleRect. Intersect the
        // window's content bounds explicitly, including in headless snapshots.
        let viewport = window?.contentView.map { document.convert($0.bounds, from: $0) } ?? visible
        let intersection = visible.intersection(viewport)
        let frame = intersection.isNull ? CGRect.zero : intersection
        if surface.frame != frame {
            surface.frame = frame
            surface.needsDisplay = true
        }
    }
}

@MainActor private final class CodeSurface: NSView {
    var code: CodeDocument {
        didSet {
            selection = 0..<0
            needsDisplay = true
        }
    }
    var ink: ThemeColor { didSet { needsDisplay = true } }
    var write: @MainActor (String) -> Bool
    private var anchor: Int?
    private var selection = 0..<0 { didSet { needsDisplay = true } }

    init(source: String, ink: ThemeColor, write: @escaping @MainActor (String) -> Bool) {
        code = CodeDocument(source)
        self.ink = ink
        self.write = write
        super.init(frame: .zero)
        setAccessibilityElement(true)
        setAccessibilityRole(.textArea)
    }

    required init?(coder: NSCoder) { return nil }

    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .iBeam) }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        let lineHeight = CodeDocument.lineHeight
        let inset = CodeDocument.inset
        // AppKit's document coordinates are bottom-up.
        let top = code.size.height - frame.maxY - inset.y
        let first = max(0, Int(((top + bounds.maxY - dirtyRect.maxY) / lineHeight).rounded(.down)))
        let last = min(code.rowCount, Int(((top + bounds.maxY - dirtyRect.minY) / lineHeight).rounded(.up)))
        guard first < last else { return }
        let originX = frame.minX - inset.x
        let left = originX + dirtyRect.minX
        let right = originX + dirtyRect.maxX
        guard left < right else { return }
        let font = CodeDocument.font
        NSColor(srgbRed: ink.red, green: ink.green, blue: ink.blue, alpha: ink.alpha).setFill()
        context.textMatrix = .identity
        for row in first..<last {
            let bottom = bounds.height + top - CGFloat(row + 1) * lineHeight
            let baseline = bottom + (lineHeight - font.ascender - font.descender) / 2
            for segment in code.segments(row: row, visible: left..<right) {
                if segment.x >= right { break }
                for run in code.runs(segment) where run.end >= left && run.x < right {
                    let selected = selection.clamped(to: run.bytes)
                    if !selected.isEmpty {
                        let a = run.x(at: selected.lowerBound) - originX
                        let b = run.x(at: selected.upperBound) - originX
                        context.saveGState()
                        NSColor.selectedTextBackgroundColor.setFill()
                        context.fill(CGRect(x: min(a, b), y: bottom, width: abs(b - a), height: lineHeight))
                        context.restoreGState()
                    }
                    if let line = run.line {
                        context.textPosition = CGPoint(x: run.x - originX, y: baseline)
                        CTLineDraw(line, context)
                    }
                }
            }
        }
    }

    private func byte(at event: NSEvent) -> Int {
        let point = convert(event.locationInWindow, from: nil)
        let inset = CodeDocument.inset
        let y = code.size.height - (frame.minY + point.y) - inset.y
        let index = max(0, Int((y / CodeDocument.lineHeight).rounded(.down)))
        guard let row = code.row(index) else { return code.source.utf8.count }
        let x = frame.minX + point.x - inset.x
        guard x > 0 else { return row.lowerBound }
        for segment in code.segments(row: index, visible: x..<x.nextUp) {
            for run in code.runs(segment) where x <= run.end { return run.byte(at: x) }
            if segment.x > x { break }
        }
        return row.upperBound
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        let offset = byte(at: event)
        anchor = event.modifierFlags.contains(.shift) ? selection.lowerBound : offset
        if event.clickCount == 2 {
            let bytes = code.source.utf8
            func word(_ byte: UInt8) -> Bool {
                switch byte {
                case 48...57, 65...90, 95, 97...122, 128...255: true
                default: false
                }
            }
            let before = bytes.prefix(offset).reversed().prefix(while: word).count
            let after = bytes.dropFirst(offset).prefix(while: word).count
            selection = (offset - before)..<(offset + after)
        } else if let anchor {
            selection = min(anchor, offset)..<max(anchor, offset)
        }
    }

    override func mouseDragged(with event: NSEvent) {
        guard let anchor else { return }
        _ = autoscroll(with: event)
        let offset = byte(at: event)
        selection = min(anchor, offset)..<max(anchor, offset)
    }

    override func selectAll(_ sender: Any?) { selection = 0..<code.source.utf8.count }

    @objc func copy(_ sender: Any?) {
        if !selection.isEmpty { _ = write(selectedSource) }
    }

    private var selectedSource: String {
        String(decoding: code.source.utf8.dropFirst(selection.lowerBound).prefix(selection.count), as: UTF8.self)
    }

    override func accessibilityValue() -> Any? { code.source }
    override func accessibilitySelectedText() -> String? { selectedSource }

    override func accessibilityAttributedString(for range: NSRange) -> NSAttributedString? {
        let text = code.source as NSString
        guard range.location >= 0, range.length >= 0, range.location <= text.length,
              range.length <= text.length - range.location else { return nil }
        return NSAttributedString(string: text.substring(with: range), attributes: [.font: CodeDocument.font])
    }
}
