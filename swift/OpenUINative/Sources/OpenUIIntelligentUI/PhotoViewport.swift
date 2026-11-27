import AppKit
import SwiftUI

struct PhotoPlacement: Equatable {
    let size: PhotoSize
    let scope: PhotoBudget.Scope

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.size == rhs.size && lhs.scope === rhs.scope
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
        private var visibility: NSKeyValueObservation?

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
            if superview == nil || last?.scope.belongs(to: window) == false {
                last?.scope.cancel()
            }
            let notifications = NotificationCenter.default
            notifications.removeObserver(self)
            visibility = nil
            if let window {
                // Ordering the window back in, or unhiding the app, posts no
                // notification the probe can count on. A scope that lapsed
                // while the window was hidden is replaced on this refresh.
                visibility = window.observe(\.isVisible, options: [.new]) { [weak self] _, change in
                    guard change.newValue == true else { return }
                    Task { @MainActor in self?.refresh() }
                }
                for name in [NSWindow.didResizeNotification, NSWindow.didMiniaturizeNotification,
                             NSWindow.didDeminiaturizeNotification, NSWindow.didChangeOcclusionStateNotification,
                             NSWindow.didBecomeKeyNotification, NSWindow.willCloseNotification] {
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
            visibility = nil
            update = nil
            last?.scope.cancel()
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
                    let scope: PhotoBudget.Scope
                    if let previous = last?.scope, previous.belongs(to: window), previous.isCurrent {
                        scope = previous
                    } else {
                        scope = PhotoBudget.inWindow(window).makeScope()
                    }
                    next = PhotoPlacement(size: size, scope: scope)
                } else {
                    next = nil
                }
                guard next != last else { return }
                if last?.scope !== next?.scope { last?.scope.cancel() }
                last = next
                update(next)
            }
        }
    }
}
