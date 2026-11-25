import AppKit
import Testing

extension CollectionDrawingTests {
    // Flip on a ci/** branch to run both probes. Keep false on main.
    static let runDisplayProbesByDefault = true

    @MainActor static func canRunDisplayProbe(_ variable: String, label: String) -> Bool {
        // Like the existing polar probe, use an early return so the package
        // inventory audit still sees the test start when its body is disabled.
        guard ProcessInfo.processInfo.environment[variable].map({ $0 == "1" }) ?? runDisplayProbesByDefault else {
            return false
        }
        guard !NSScreen.screens.isEmpty else {
            print("[\(label)] SKIP: No macOS display is available")
            return false
        }
        return true
    }
}

@MainActor struct RunnerDisplaySession {
    private let app = NSApplication.shared
    private let previousPolicy: NSApplication.ActivationPolicy
    private let previousFrontApp = NSWorkspace.shared.frontmostApplication

    init() {
        previousPolicy = app.activationPolicy()
        app.setActivationPolicy(.regular)
    }

    func restore() {
        app.setActivationPolicy(previousPolicy)
        previousFrontApp?.activate(options: [])
    }

    func present(_ fixture: CollectionWindow) async throws {
        let window = fixture.window
        let screen = try #require(NSScreen.screens.first, "No macOS display is available")
        let frame = screen.visibleFrame
        window.setFrameOrigin(CGPoint(x: frame.midX - window.frame.width / 2,
                                      y: frame.midY - window.frame.height / 2))
        app.activate()
        window.makeKeyAndOrderFront(nil)
        window.orderFrontRegardless()
        let deadline = ContinuousClock.now + .seconds(2)
        repeat {
            await fixture.settle()
            drainEvents()
            if isFrontmost(window) { return }
        } while ContinuousClock.now < deadline
        throw CollectionInputFailure(description: "Could not make probe window key and frontmost; \(describe(window))")
    }

    func isFrontmost(_ window: NSWindow) -> Bool {
        app.isActive && window.isVisible && window.isOnActiveSpace && window.screen != nil
            && window.isKeyWindow && window.isMainWindow && app.orderedWindows.first === window
    }

    // The SwiftPM test process doesn't call NSApplication.run(). Deliver only
    // events already queued by AppKit, including the activation notifications.
    func drainEvents(observe: (NSEvent) -> Void = { _ in }) {
        for _ in 0..<64 {
            guard let event = app.nextEvent(matching: .any, until: .distantPast, inMode: .default, dequeue: true) else { break }
            observe(event)
            app.sendEvent(event)
        }
    }

    func describe(_ window: NSWindow) -> String {
        "window frame=\(window.frame) key=\(window.isKeyWindow) main=\(window.isMainWindow) "
            + "appActive=\(app.isActive) appFrontWindow=\(app.orderedWindows.first === window) "
            + "visible=\(window.isVisible) onActiveSpace=\(window.isOnActiveSpace) "
            + "screen=\(String(describing: window.screen?.frame)) backingScale=\(window.backingScaleFactor)"
    }
}
