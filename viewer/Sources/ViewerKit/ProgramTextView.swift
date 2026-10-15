import AppKit
import SwiftUI

/// The program's text: what the shown step has received in full color, the
/// latest chunk highlighted, and the rest faded. AppKit's text view only
/// recolors on each step, which keeps large programs cheap.
struct ProgramTextView: NSViewRepresentable {
    let text: String
    /// Where the shown step's text ends, in UTF-16 units.
    let received: Int
    /// Where the step before it ended: the latest chunk is in between.
    let previous: Int

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSTextView.scrollableTextView()
        let textView = scrollView.documentView as! NSTextView
        textView.isEditable = false
        textView.isSelectable = true
        textView.font = .monospacedSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        textView.textContainerInset = NSSize(width: 6, height: 6)
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = scrollView.documentView as? NSTextView, let storage = textView.textStorage else { return }
        if textView.string != text {
            textView.string = text
        }
        let length = storage.length
        let end = min(received, length)
        let start = min(previous, end)
        storage.beginEditing()
        storage.removeAttribute(.backgroundColor, range: NSRange(location: 0, length: length))
        storage.addAttribute(.foregroundColor, value: NSColor.tertiaryLabelColor, range: NSRange(location: 0, length: length))
        storage.addAttribute(.foregroundColor, value: NSColor.labelColor, range: NSRange(location: 0, length: end))
        if end > start {
            storage.addAttribute(
                .backgroundColor,
                value: NSColor.systemOrange.withAlphaComponent(0.3),
                range: NSRange(location: start, length: end - start)
            )
        }
        storage.endEditing()
        textView.scrollRangeToVisible(NSRange(location: end, length: 0))
    }
}
