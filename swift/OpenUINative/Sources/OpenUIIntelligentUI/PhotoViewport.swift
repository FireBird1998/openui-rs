import AppKit
import SwiftUI

struct PhotoPlacement: Equatable {
    let size: PhotoSize
    let budget: PhotoBudget

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.size == rhs.size && lhs.budget === rhs.budget
    }
}

/// AppKit's visible rect includes the hosting window and enclosing clip
/// views, including an NSScrollView installed by the host app.
struct PhotoViewport: NSViewRepresentable {
    let cover: Bool
    let update: (PhotoPlacement?) -> Void

    func makeNSView(context: Context) -> Probe { Probe() }

    func updateNSView(_ view: Probe, context: Context) {
        view.cover = cover
        view.update = update
        view.refresh()
    }

    static func dismantleNSView(_ view: Probe, coordinator: ()) { view.stop() }

    final class Probe: NSView {
        var cover = false
        var update: ((PhotoPlacement?) -> Void)?
        private var observationPending = false
        private var last: PhotoPlacement?

        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            observe()
        }

        override func viewDidMoveToSuperview() {
            super.viewDidMoveToSuperview()
            observe()
        }

        override func viewDidChangeBackingProperties() {
            super.viewDidChangeBackingProperties()
            refresh()
        }

        override func layout() {
            super.layout()
            refresh()
        }

        private func observe() {
            let notifications = NotificationCenter.default
            notifications.removeObserver(self)
            if let window {
                for name in [NSWindow.didResizeNotification, NSWindow.didMiniaturizeNotification,
                             NSWindow.didDeminiaturizeNotification, NSWindow.didChangeOcclusionStateNotification] {
                    notifications.addObserver(self, selector: #selector(changed), name: name, object: window)
                }
                var ancestor = superview
                for _ in 0..<PhotoLimits.viewportAncestors {
                    guard let view = ancestor else { break }
                    if let clip = view as? NSClipView {
                        clip.postsBoundsChangedNotifications = true
                        notifications.addObserver(self, selector: #selector(changed),
                                                  name: NSView.boundsDidChangeNotification, object: clip)
                    }
                    ancestor = view.superview
                }
            }
            refresh()
        }

        @objc private func changed(_ notification: Notification) { refresh() }

        func stop() {
            NotificationCenter.default.removeObserver(self)
            update = nil
            last = nil
        }

        func refresh() {
            guard update != nil, !observationPending else { return }
            observationPending = true
            Task { @MainActor [weak self] in
                guard let self else { return }
                observationPending = false
                guard let update else { return }
                let next: PhotoPlacement?
                if let window, window.isVisible, !window.isMiniaturized, !isHiddenOrHasHiddenAncestor,
                   let content = window.contentView,
                   !visibleRect.intersection(convert(content.bounds, from: content)).isEmpty,
                   let size = PhotoSize(points: bounds.size, scale: window.backingScaleFactor, cover: cover) {
                    next = PhotoPlacement(size: size, budget: PhotoBudget.inWindow(window))
                } else {
                    next = nil
                }
                guard next != last else { return }
                last = next
                update(next)
            }
        }
    }
}
