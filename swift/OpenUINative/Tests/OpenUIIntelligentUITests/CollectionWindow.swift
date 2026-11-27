// Mount the real library and deliver AppKit input, as in OpenUINative's ClickTests.
import AppKit
import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import Testing

@Suite(.serialized) struct CollectionDrawingTests {}

@MainActor func layoutOnMainQueue(_ host: NSView) async {
    await withCheckedContinuation { (laidOut: CheckedContinuation<Void, Never>) in
        DispatchQueue.main.async {
            host.layoutSubtreeIfNeeded()
            host.window?.displayIfNeeded()
            laidOut.resume()
        }
    }
}

// Swift Testing prints a caught error's description on the CI issue line.
struct CollectionInputFailure: Error, CustomStringConvertible {
    let description: String
}

/// Whether this process has a display that is awake and producing frames.
/// SwiftUI animations and NSScrollView's continuous scrolling only advance on
/// display frames, so a screen that exists but is asleep never moves them.
/// Decided once per process.
@MainActor enum DisplayFrames {
    enum State: Equatable { case absent, asleep, ticking }

    static let noDisplayReason: Comment = "No macOS display is available"
    static let asleepReason: Comment = "display asleep, no frames"

    private static var cached: State?

    static var state: State {
        if let cached { return cached }
        let decided: State
        if NSScreen.screens.isEmpty {
            decided = .absent
        } else if mainDisplayIsAsleep || ticks(for: .milliseconds(150), stopAtFirst: true) == 0 {
            decided = .asleep
        } else {
            decided = .ticking
        }
        cached = decided
        return decided
    }

    /// Animated scrolling runs only where frames tick.
    static var animates: Bool { state == .ticking }

    static var mainDisplayIsAsleep: Bool { CGDisplayIsAsleep(CGMainDisplayID()) != 0 }

    private final class Counter: NSObject {
        var ticks = 0
        @objc func tick(_ link: CADisplayLink) { ticks += 1 }
    }

    /// Count the main screen's display-link callbacks. The run loop runs in a
    /// private mode, so no other main-queue work runs inside this wait.
    static func ticks(for duration: Duration, stopAtFirst: Bool = false) -> Int {
        guard let screen = NSScreen.screens.first else { return 0 }
        let counter = Counter()
        let link = screen.displayLink(target: counter, selector: #selector(Counter.tick(_:)))
        let mode = RunLoop.Mode("org.openui.tests.display-frames")
        link.add(to: .main, forMode: mode)
        defer { link.invalidate() }
        let seconds = Double(duration.components.seconds) + Double(duration.components.attoseconds) / 1e18
        let deadline = Date(timeIntervalSinceNow: seconds)
        while Date() < deadline, !(stopAtFirst && counter.ticks > 0) {
            RunLoop.main.run(mode: mode, before: deadline)
        }
        return counter.ticks
    }
}

/// Skips a test unless a display is awake and producing frames, naming which
/// of the two is missing.
struct RequiresDisplayFrames: TestTrait {
    func prepare(for test: Test) async throws {
        switch await DisplayFrames.state {
        case .absent: try await ConditionTrait.disabled(DisplayFrames.noDisplayReason).prepare(for: test)
        case .asleep: try await ConditionTrait.disabled(DisplayFrames.asleepReason).prepare(for: test)
        case .ticking: break
        }
    }
}

extension Trait where Self == RequiresDisplayFrames {
    static var requiresDisplay: Self { RequiresDisplayFrames() }
}

@MainActor final class CollectionWindow {
    let live: LiveProgram
    let window: NSWindow
    let host: NSHostingView<AnyView>
    private(set) var frames: [String: CGRect] = [:]

    init(_ program: String, width: CGFloat = 320, height: CGFloat = 200, scrollable: Bool = false, streaming: Bool = false,
         mode: IntelligentUIMode? = nil,
         colorSpace: WindowColorSpace? = nil,
         component: @escaping @MainActor @Sendable (Element, String) -> AnyView? = { _, _ in nil }) async throws {
        let animates = DisplayFrames.animates
        live = try LiveProgram(library: try await Core().intelligentUILibrary())
        try await live.show(response: program, streaming: streaming)
        #expect(live.errors.map(\.message) == [])
        _ = NSApplication.shared
        host = NSHostingView(rootView: AnyView(EmptyView()))
        window = NSWindow(contentRect: CGRect(x: 60, y: 60, width: width, height: height),
                          styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        if let colorSpace { window.colorSpace = colorSpace.colorSpace }
        let drawing = ElementDrawing(component: component, chrome: { [weak self] view, _, path in
            AnyView(view.onGeometryChange(for: CGRect.self) { $0.frame(in: .named("window")) } action: {
                self?.frames[path] = $0
            })
        })
        let page = ElementView(live: live).environment(\.elementDrawing, drawing)
        let content = scrollable
            ? AnyView(ScrollView { page.frame(minHeight: 400, alignment: .topLeading) })
            : AnyView(page)
        host.rootView = AnyView(content
            .intelligentUIMode(mode)
            .environment(\.accessibilityEnabled, true)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).coordinateSpace(name: "window")
            .transaction { if !animates { $0.animation = nil } })
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        window.orderFrontRegardless()
        await settle()
    }

    func settle() async {
        await layoutOnMainQueue(host)
    }

    // Window and browser updates can arrive between layout passes.
    @discardableResult
    func waitForLayout(for timeout: Duration, until ready: () throws -> Bool) async throws -> Bool {
        let clock = ContinuousClock()
        let deadline = clock.now + timeout
        while true {
            try Task.checkCancellation()
            if try ready() { return true }
            let now = clock.now
            guard now < deadline else { return false }
            try await clock.sleep(until: min(deadline, now + .milliseconds(20)))
            await settle()
        }
    }

    func rect(_ path: String) throws -> CGRect { try #require(frames[path]) }

    var scrollView: NSScrollView? {
        var pending = [host as NSView]
        while let view = pending.popLast() {
            if let scroll = view as? NSScrollView { return scroll }
            pending.append(contentsOf: view.subviews)
        }
        return nil
    }

    var accessibleButtons: [AnyObject] {
        var pending: [AnyObject] = [host]
        var buttons: [AnyObject] = []
        while let node = pending.popLast() {
            if node.accessibilityRole?() == .button {
                buttons.append(node)
            }
            pending.append(contentsOf: (node.accessibilityChildren?() ?? []).map { $0 as AnyObject })
        }
        return buttons
    }

    var buttonLabels: [String] {
        accessibleButtons.map { $0.accessibilityLabel?() ?? $0.accessibilityTitle?() ?? "" }
    }

    func click(_ point: CGPoint, throughApplication: Bool = false) async throws {
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try #require(NSEvent.mouseEvent(with: type, location: host.convert(point, to: nil),
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
            if throughApplication { NSApp.sendEvent(event) } else { window.sendEvent(event) }
        }
        await settle()
    }
}
