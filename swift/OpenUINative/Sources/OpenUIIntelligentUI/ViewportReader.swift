import AppKit
import SwiftUI

struct ViewportReading: Equatable {
    let range: Range<CGFloat>
    let selectedY: CGFloat?
}

/// Read visibility from the mounted host, including ancestor scroll clips
/// and the window. The probe never participates in drawing or mouse events.
struct ViewportReader: NSViewRepresentable {
    var changed: (ViewportReading?) -> Void

    func makeNSView(context: Context) -> Probe {
        let probe = Probe()
        probe.changed = changed
        return probe
    }

    func updateNSView(_ probe: Probe, context: Context) {
        probe.changed = changed
        probe.scheduleRead()
    }

    final class Probe: NSView {
        var changed: ((ViewportReading?) -> Void)?
        private var previous: ViewportReading?
        private var pending = false

        override var isFlipped: Bool { true }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override var frame: NSRect {
            didSet { scheduleRead() }
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            let notifications = NotificationCenter.default
            notifications.removeObserver(self)
            if let window {
                notifications.addObserver(self, selector: #selector(viewportChanged),
                                          name: NSWindow.didResizeNotification, object: window)
                var ancestor = superview
                while let view = ancestor {
                    if let clip = view as? NSClipView {
                        clip.postsBoundsChangedNotifications = true
                        notifications.addObserver(self, selector: #selector(viewportChanged),
                                                  name: NSView.boundsDidChangeNotification, object: clip)
                    }
                    ancestor = view.superview
                }
            }
            scheduleRead()
        }

        override func setFrameOrigin(_ newOrigin: NSPoint) {
            super.setFrameOrigin(newOrigin)
            scheduleRead()
        }

        override func setFrameSize(_ newSize: NSSize) {
            super.setFrameSize(newSize)
            scheduleRead()
        }

        @objc private func viewportChanged() {
            scheduleRead()
        }

        func scheduleRead() {
            guard !pending else { return }
            pending = true
            Task { @MainActor [weak self] in
                guard let self else { return }
                pending = false
                let visibility = window.map { window in
                    let visible = visibleRect.intersection(convert(window.contentLayoutRect, from: nil))
                    let range = visible.isEmpty ? 0..<0 : visible.minY..<visible.maxY
                    return ViewportReading(range: range, selectedY: selectionY(in: window))
                }
                guard visibility != previous else { return }
                previous = visibility
                changed?(visibility)
            }
        }

        // Keep a single active selection mounted. Moving the row out of the
        // clip then leaves the native range, responder, and Copy action intact.
        private func selectionY(in window: NSWindow) -> CGFloat? {
            guard let document = enclosingScrollView?.documentView,
                  let responder = window.firstResponder as? NSView,
                  responder.isDescendant(of: document),
                  responder.accessibilitySelectedTextRange().length > 0 else { return nil }
            return convert(responder.bounds, from: responder).midY
        }
    }
}
