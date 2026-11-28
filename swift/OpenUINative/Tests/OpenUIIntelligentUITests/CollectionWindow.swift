// Mount the real library and deliver AppKit input, as in OpenUINative's ClickTests.
import AppKit
// @testable for the internal Reduce Motion seam (ReduceMotion.swift).
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import SwiftUI
import SwiftTestSupport
import Synchronization
import Testing

@Suite(.serialized) struct CollectionDrawingTests {}

@MainActor func layoutOnMainQueue(_ host: NSView, waitingFor operation: String) async {
    NativeTestWatchdog.beginWaiting(operation)
    defer { NativeTestWatchdog.finishedWaiting() }
    await withCheckedContinuation { (laidOut: CheckedContinuation<Void, Never>) in
        DispatchQueue.main.async {
            host.layoutSubtreeIfNeeded()
            host.window?.displayIfNeeded()
            laidOut.resume()
        }
    }
}

extension NSWindow {
    @MainActor func closeForTest() {
        NativeTestWatchdog.beginWaiting("AppKit to close the test window")
        defer { NativeTestWatchdog.finishedWaiting() }
        close()
    }
}

// Swift Testing prints a caught error's description on the CI issue line.
struct CollectionInputFailure: Error, CustomStringConvertible {
    let description: String
}

/// Whether this process has a display that is awake and producing frames.
/// SwiftUI animations and NSScrollView's continuous scrolling only advance on
/// display frames, so a screen that exists but is asleep never moves them.
///
/// Sampled afresh on every read: when each display test starts (through its
/// `.requiresDisplay` trait), when a CollectionWindow mounts, and in the
/// animated tests' own checks. It used to be decided once per process, so a
/// display that fell asleep partway through a run left the later animated
/// Carousel tests on the animated path with no frames, failing. Now the
/// display tests that start after the sleep are cancelled as they start,
/// naming it, and the test running when the display falls asleep fails once,
/// naming that instead (`RequiresDisplayFrames`).
@MainActor enum DisplayFrames {
    enum State: Equatable { case absent, asleep, ticking }

    nonisolated static let noDisplayReason: Comment = "No macOS display is available"
    nonisolated static let asleepReason: Comment = "display asleep, no frames"
    nonisolated static let sleptDuringRun: Comment = "display slept during run"

    static var state: State {
        if NSScreen.screens.isEmpty { return .absent }
        let real: () -> State = {
            mainDisplayIsAsleep || ticks(for: .milliseconds(150), stopAtFirst: true) == 0 ? .asleep : .ticking
        }
        return SimulatedSamples.next(real)
    }

    /// Animated scrolling runs only where frames tick.
    static var animates: Bool { state == .ticking }

    static var mainDisplayIsAsleep: Bool { CGDisplayIsAsleep(CGMainDisplayID()) != 0 }

    /// Whether the display is asleep now, readable from any thread: Swift
    /// Testing calls a known-issue matcher wherever the issue is recorded.
    nonisolated static var asleepNow: Bool {
        CGDisplayIsAsleep(CGMainDisplayID()) != 0 || SimulatedSamples.lastSaidAsleep
    }

    /// A test-only seam that stands in for the display changing during a run.
    /// OPENUI_TEST_DISPLAY_SAMPLES is a comma-separated list, one entry per
    /// sample of `state` in order: `awake` answers ticking, `asleep` answers
    /// asleep and `real` samples the display. Samples past the list's end are
    /// real. "real,real,asleep" is an awake display that falls asleep at the
    /// third sample; "awake,awake" on a sleeping display is one that was
    /// awake when the first display test began. Each sample is printed while
    /// the seam is set. Nothing sets it in CI, and the gate refuses to run
    /// while it is set (TESTING.md).
    nonisolated enum SimulatedSamples {
        private struct Samples {
            let script: [String]? = ProcessInfo.processInfo.environment["OPENUI_TEST_DISPLAY_SAMPLES"]
                .map { $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) } }
            var taken = 0
            var lastSaidAsleep = false
        }

        private static let samples = Mutex(Samples())

        @MainActor static func next(_ real: () -> State) -> State {
            let (script, index) = samples.withLock { samples in
                samples.taken += 1
                return (samples.script, samples.taken)
            }
            guard let script else { return real() }
            let entry = index <= script.count ? script[index - 1] : "real"
            let state: State = switch entry {
            case "awake": .ticking
            case "asleep": .asleep
            default: real()
            }
            samples.withLock { $0.lastSaidAsleep = state == .asleep }
            let test = Test.current?.name ?? "outside a test"
            try? FileHandle.standardError.write(contentsOf: Data(
                "[display-sample] \(index): \(state) (\(entry == "real" || index > script.count ? "real" : "simulated")) in \(test)\n".utf8))
            return state
        }

        static var lastSaidAsleep: Bool { samples.withLock { $0.lastSaidAsleep } }
    }

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
/// of the two is missing. Swift Testing runs `prepare` when it plans the run,
/// so the scope samples the display again as each test starts and cancels the
/// test with the same reason if it has fallen asleep since. The gate accepts
/// either (scripts/check-swift-tests.py, `frame_skips_after_run`).
///
/// If the display falls asleep while the test runs, what fails next is the
/// frames stopping, not the code under test: each issue recorded while the
/// display is asleep becomes a known issue, and the test records one failure,
/// "display slept during run", in their place. Issues recorded while the
/// display is awake still fail the test as usual.
struct RequiresDisplayFrames: TestTrait, TestScoping {
    func prepare(for test: Test) async throws {
        switch await DisplayFrames.state {
        case .absent: try await ConditionTrait.disabled(DisplayFrames.noDisplayReason).prepare(for: test)
        case .asleep: try await ConditionTrait.disabled(DisplayFrames.asleepReason).prepare(for: test)
        case .ticking: break
        }
    }

    private final class Slept: Sendable {
        let value = Mutex(false)
    }

    func provideScope(for test: Test, testCase: Test.Case?,
                      performing function: @Sendable () async throws -> Void) async throws {
        // Swift Testing runs `prepare` when it plans the run, before any test
        // starts, so sample again here, as this test starts.
        let reason: Comment? = switch await DisplayFrames.state {
        case .absent: DisplayFrames.noDisplayReason
        case .asleep: DisplayFrames.asleepReason
        case .ticking: nil
        }
        if let reason {
            // A cancelled case of a parameterized test reports no reason, so
            // name it in a known issue first.
            if test.isParameterized { withKnownIssue(reason) { Issue.record(reason) } }
            try Test.cancel(reason)
        }
        let slept = Slept()
        try await withKnownIssue(DisplayFrames.sleptDuringRun, isIntermittent: true) {
            try await function()
        } matching: { _ in
            guard DisplayFrames.asleepNow else { return false }
            slept.value.withLock { $0 = true }
            return true
        }
        if slept.value.withLock({ $0 }) {
            Issue.record(DisplayFrames.sleptDuringRun)
        }
    }
}

extension Trait where Self == RequiresDisplayFrames {
    static var requiresDisplay: Self { RequiresDisplayFrames() }
}

/// Whether the login session's screen is locked. Printed with the display
/// facts as information only: no test skips on it.
@MainActor enum SessionLock {
    static var isLocked: Bool {
        guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
        return (session["CGSSessionScreenIsLocked"] as? Bool) == true
    }
}

/// Whether AppKit minimises a window in this process, measured once with a
/// small probe window. Some test hosts, such as an unbundled `swift test`
/// process on a desktop Mac, leave every minimise pending; GitHub's macOS
/// runner minimises. `scripts/check-swift-tests.py` runs the same probe.
@MainActor enum WindowMinimise {
    static let refusedReason: Comment = "AppKit refused to minimise a window in this process (unbundled test host)"

    private static var cached: Bool?

    static var works: Bool {
        get async {
            if let cached { return cached }
            let decided = await probe()
            cached = decided
            return decided
        }
    }

    private static func probe() async -> Bool {
        NativeTestWatchdog.beginWaiting("AppKit to minimise the probe window")
        defer { NativeTestWatchdog.finishedWaiting() }
        _ = NSApplication.shared
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 48, height: 48),
                              styleMask: [.titled, .miniaturizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.orderFrontRegardless()
        // Ask only once the window server shows the window: a window that is
        // not on screen yet reports itself minimised at once.
        let clock = ContinuousClock()
        let shown = clock.now + .seconds(1)
        while !window.occlusionState.contains(.visible), clock.now < shown {
            try? await Task.sleep(for: .milliseconds(20))
        }
        try? await Task.sleep(for: .milliseconds(100))
        window.miniaturize(nil)
        let deadline = clock.now + .seconds(3)
        while !window.isMiniaturized, clock.now < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
        let minimised = window.isMiniaturized
        if minimised { window.deminiaturize(nil) }
        return minimised
    }
}

/// Skips a test that minimises a window when AppKit refuses to minimise the
/// probe window in this process, naming that.
struct RequiresWindowMinimise: TestTrait {
    func prepare(for test: Test) async throws {
        if await !WindowMinimise.works {
            try await ConditionTrait.disabled(WindowMinimise.refusedReason).prepare(for: test)
        }
    }
}

extension Trait where Self == RequiresWindowMinimise {
    static var requiresWindowMinimise: Self { RequiresWindowMinimise() }
}

@MainActor final class CollectionWindow {
    let live: LiveProgram
    let window: NSWindow
    let host: NSHostingView<AnyView>
    private(set) var frames: [String: CGRect] = [:]
    private var observedDrawing = false
    private var drawnRoot: Origin?

    init(_ program: String, width: CGFloat = 320, height: CGFloat = 200, scrollable: Bool = false, streaming: Bool = false,
         mode: IntelligentUIMode? = nil, makeKey: Bool = true,
         colorSpace: WindowColorSpace? = nil, reduceMotion: Bool? = nil,
         observeProgramDrawing: Bool = false,
         component: @escaping @MainActor @Sendable (Element, String) -> AnyView? = { _, _ in nil }) async throws {
        NativeTestWatchdog.beginWaiting("the display link to show whether CollectionWindow animates")
        defer { NativeTestWatchdog.finishedWaiting() }
        let animates = DisplayFrames.animates
        NativeTestWatchdog.waitingFor("Core to load the Intelligent UI library for CollectionWindow")
        live = try LiveProgram(library: try await Core().intelligentUILibrary())
        NativeTestWatchdog.waitingFor("Core to show the CollectionWindow program")
        try await live.show(response: program, streaming: streaming)
        #expect(live.errors.map(\.message) == [])
        NativeTestWatchdog.waitingFor("AppKit to create CollectionWindow and its hosting view")
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
        let elementView = ElementView(live: live).environment(\.elementDrawing, drawing)
        let page = observeProgramDrawing
            ? AnyView(elementView.modifier(DrawingCheckpoint(live: live) { [weak self] origin in
                self?.drawnRoot = origin
                self?.observedDrawing = true
            }))
            : AnyView(elementView)
        let content = scrollable
            ? AnyView(ScrollView { page.frame(minHeight: 400, alignment: .topLeading) })
            : AnyView(page)
        NativeTestWatchdog.waitingFor("AppKit to mount CollectionWindow's root view")
        host.rootView = AnyView(content
            .intelligentUIMode(mode)
            .environment(\.accessibilityEnabled, true)
            // Nil follows the system; a motion test pins its path here.
            .environment(\.reduceMotionOverride, reduceMotion)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).coordinateSpace(name: "window")
            .transaction { if !animates { $0.animation = nil } })
        window.contentView = host
        NativeTestWatchdog.waitingFor("AppKit to order CollectionWindow onscreen")
        if makeKey { window.makeKeyAndOrderFront(nil) }
        window.orderFrontRegardless()
        if observeProgramDrawing {
            try await waitForProgramDrawing()
        } else {
            await settle()
        }
    }

    func waitForProgramDrawing() async throws {
        let root = live.root?.origin
        try await waitForCollectionEvent("the current program drawing") {
            host.layoutSubtreeIfNeeded()
            return observedDrawing && drawnRoot == root && (root == nil || frames["/"] != nil)
        }
    }

    func settle(waitingFor operation: String = "AppKit to lay out and display CollectionWindow on the main queue") async {
        await layoutOnMainQueue(host, waitingFor: operation)
    }

    // Window and browser updates can arrive between layout passes.
    @discardableResult
    func waitForLayout(_ operation: String, for timeout: Duration, until ready: () throws -> Bool) async throws -> Bool {
        NativeTestWatchdog.beginWaiting(operation)
        defer { NativeTestWatchdog.finishedWaiting() }
        let clock = ContinuousClock()
        let deadline = clock.now + timeout
        while true {
            try Task.checkCancellation()
            if try ready() { return true }
            let now = clock.now
            guard now < deadline else { return false }
            try await clock.sleep(until: min(deadline, now + .milliseconds(20)))
            await settle(waitingFor: "AppKit layout on the main queue while waiting for \(operation)")
        }
    }

    func frame(at path: String) -> CGRect? { frames[path] }

    func rect(_ path: String) throws -> CGRect { try #require(frames[path]) }

    var scrollView: NSScrollView? {
        NativeTestWatchdog.beginWaiting("AppKit to find the CollectionWindow scroll view")
        defer { NativeTestWatchdog.finishedWaiting() }
        var pending = [host as NSView]
        while let view = pending.popLast() {
            if let scroll = view as? NSScrollView { return scroll }
            pending.append(contentsOf: view.subviews)
        }
        return nil
    }

    var accessibleButtons: [AnyObject] {
        NativeTestWatchdog.beginWaiting("AppKit to find the CollectionWindow accessibility buttons")
        defer { NativeTestWatchdog.finishedWaiting() }
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
        let buttons = accessibleButtons
        NativeTestWatchdog.beginWaiting("AppKit to read the CollectionWindow button labels")
        defer { NativeTestWatchdog.finishedWaiting() }
        return buttons.map { $0.accessibilityLabel?() ?? $0.accessibilityTitle?() ?? "" }
    }

    func click(_ point: CGPoint, throughApplication: Bool = false, count: Int = 1) async throws {
        NativeTestWatchdog.waitingFor("AppKit to dispatch the CollectionWindow click at \(point)")
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try #require(NSEvent.mouseEvent(with: type, location: host.convert(point, to: nil),
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                context: nil, eventNumber: 0, clickCount: count, pressure: 1))
            if throughApplication { NSApp.sendEvent(event) } else { window.sendEvent(event) }
        }
        await settle()
    }
}

private struct DrawingCheckpoint: ViewModifier {
    let live: LiveProgram
    let didDraw: (Origin?) -> Void

    func body(content: Content) -> some View {
        content.onChange(of: live.root?.origin, initial: true) { _, origin in didDraw(origin) }
    }
}

/// Layout, pixels and server logs are snapshots rather than async sequences.
@MainActor func waitForCollectionEvent(_ event: String, watchdog: Duration = .seconds(20),
                                       untilObserved: () throws -> Bool) async throws {
    NativeTestWatchdog.beginWaiting(event)
    defer { NativeTestWatchdog.finishedWaiting() }
    let deadline = ContinuousClock.now + watchdog
    let (ticks, tick) = AsyncStream.makeStream(of: Void.self, bufferingPolicy: .bufferingNewest(1))
    let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { _ in tick.yield(()) }
    RunLoop.main.add(timer, forMode: .common)
    defer {
        timer.invalidate()
        tick.finish()
    }
    tick.yield(())
    for await _ in ticks {
        if try untilObserved() { return }
        if ContinuousClock.now >= deadline { break }
    }
    try Task.checkCancellation()
    throw CollectionInputFailure(description: "Collection event watchdog expired waiting for \(event)")
}
