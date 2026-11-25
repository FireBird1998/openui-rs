import AppKit
import CoreServices
import Testing

extension CollectionDrawingTests {
    // Flip on a ci/** branch to run both probes. Keep false on main.
    static let runDisplayProbesByDefault = true
}

enum RunnerDisplayProbe {
    case screenColour
    case realPointer(String)

    private static let childVariable = "OPENUI_DISPLAY_PROBE_CHILD"
    private static let childTimeoutSeconds = 15
    private static let completion = "child probe finished"

    // The child uses the same SwiftPM executable, filtered to the calling test.
    // For a parameterized test, only the case matching the child's prefix runs.
    @MainActor func run(_ body: () async throws -> Void) async throws {
        if let child = ProcessInfo.processInfo.environment[Self.childVariable] {
            guard child == prefix else { return }
            logDisplayProbe("\(prefix) child started (pid=\(getpid()))")
            if NSScreen.screens.isEmpty {
                logDisplayProbe("\(prefix) SKIP: No macOS display is available")
            } else {
                try await body()
            }
            logDisplayProbe("\(prefix) \(Self.completion)")
            return
        }
        // Keep the test in Swift Testing's inventory even when probes are off.
        let enabled = ProcessInfo.processInfo.environment[variable].map { $0 == "1" }
        guard enabled ?? CollectionDrawingTests.runDisplayProbesByDefault else { return }
        try await runChild()
    }

    private var label: String {
        switch self {
        case .screenColour: "screen-colour-probe"
        case .realPointer: "real-pointer-probe"
        }
    }

    var prefix: String {
        switch self {
        case .screenColour: "[\(label)]"
        case .realPointer(let kind): "[\(label)] \(kind)"
        }
    }

    private var variable: String {
        switch self {
        case .screenColour: "OPENUI_SCREEN_COLOUR_PROBE"
        case .realPointer: "OPENUI_REAL_POINTER_PROBE"
        }
    }

    private var filter: String {
        switch self {
        case .screenColour: "OnScreenColourProbeTests/windowColoursConvertToDeclaredSRGB"
        case .realPointer: "RealPointerHoverProbeTests/realPointerHover"
        }
    }

    private func runChild() async throws {
        let fileManager = FileManager.default
        let directory = fileManager.temporaryDirectory.appendingPathComponent("openui-display-probe-\(UUID())")
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer { try? fileManager.removeItem(at: directory) }
        let outputURL = directory.appendingPathComponent("output.log")
        try Data().write(to: outputURL)
        let output = try FileHandle(forWritingTo: outputURL)
        defer { try? output.close() }

        let process = Process()
        process.executableURL = Bundle.main.executableURL
        let arguments = CommandLine.arguments
        // SwiftPM can load the test bundle in its testing helper instead of
        // linking an executable test target. Reuse the bundle argument in that case.
        var childArguments: [String] = []
        if let index = arguments.firstIndex(of: "--test-bundle-path"), index + 1 < arguments.count {
            // The helper expects this pair first, before Swift Testing's options.
            childArguments += ["--test-bundle-path", arguments[index + 1]]
        }
        process.arguments = childArguments + ["--filter", filter, "--testing-library", "swift-testing"]
        var environment = ProcessInfo.processInfo.environment
        environment[Self.childVariable] = prefix
        process.environment = environment
        // A file allows both output streams to fill without blocking a child on a pipe buffer.
        process.standardOutput = output
        process.standardError = output
        try process.run()
        defer { stop(process) }

        let deadline = ContinuousClock.now + .seconds(Self.childTimeoutSeconds)
        while process.isRunning && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
        let timedOut = process.isRunning
        stop(process)

        let captured = try String(decoding: Data(contentsOf: outputURL), as: UTF8.self)
        for line in captured.split(whereSeparator: \.isNewline) {
            if line.hasPrefix(prefix) {
                logDisplayProbe(String(line))
            } else {
                // Keep child diagnostics from counting as parent test starts or summaries in the inventory gate.
                logDisplayProbe("\(prefix) child: \(line.replacingOccurrences(of: "Test ", with: "test "))")
            }
        }
        let exited = process.terminationReason == .exit
        let status = exited ? "exit status=\(process.terminationStatus) signal=none"
                            : "exit status=none signal=\(process.terminationStatus)"
        logDisplayProbe("\(prefix) child \(status)")
        if timedOut {
            throw CollectionInputFailure(description: "\(prefix) display probe child timeout (\(Self.childTimeoutSeconds) s); \(status)")
        }
        guard exited, process.terminationStatus == 0 else {
            throw CollectionInputFailure(description: "\(prefix) child failed; \(status)")
        }
        guard captured.split(whereSeparator: \.isNewline).contains("\(prefix) \(Self.completion)"[...]) else {
            throw CollectionInputFailure(description: "\(prefix) child ended before completing the probe; \(status)")
        }
    }

    private func stop(_ process: Process) {
        if process.isRunning { kill(process.processIdentifier, SIGKILL) }
        process.waitUntilExit()
    }

    fileprivate static var childPrefix: String? {
        ProcessInfo.processInfo.environment[childVariable]
    }
}

// Write each line before dispatching any AppKit event that could end the process.
func logDisplayProbe(_ line: String) {
    FileHandle.standardOutput.write(Data((line + "\n").utf8))
}

@MainActor struct RunnerDisplaySession {
    private let app = NSApplication.shared
    private let prefix: String
    private let previousPolicy: NSApplication.ActivationPolicy
    private let previousFrontApp = NSWorkspace.shared.frontmostApplication
    private let previousDelegate: (any NSApplicationDelegate)?
    private let terminationLog: RunnerDisplayTerminationLog
    private let previousAppleEventHandler: AEEventHandlerUPP?

    init(prefix: String) {
        self.prefix = prefix
        previousPolicy = app.activationPolicy()
        previousDelegate = app.delegate
        terminationLog = RunnerDisplayTerminationLog(prefix: prefix)
        app.delegate = terminationLog
        var previousHandler: AEEventHandlerUPP?
        AEGetSpecialHandler(keyPreDispatch, &previousHandler, false)
        previousAppleEventHandler = previousHandler
        let appleEventStatus = AEInstallSpecialHandler(keyPreDispatch, logDisplayAppleEvent, false)
        logDisplayProbe("\(prefix) observing termination (applicationShouldTerminate returns .terminateCancel, "
                        + "willTerminateNotification); Apple Event pre-dispatch observer status=\(appleEventStatus)")
        let changedPolicy = app.setActivationPolicy(.regular)
        logDisplayProbe("\(prefix) setActivationPolicy(.regular)=\(changedPolicy)")
    }

    func restore() {
        app.setActivationPolicy(previousPolicy)
        previousFrontApp?.activate(options: [])
        logDisplayProbe("\(prefix) termination summary: applicationShouldTerminate=\(terminationLog.requests), "
                        + "willTerminateNotification=\(terminationLog.notifications)")
        app.delegate = previousDelegate
        AERemoveSpecialHandler(keyPreDispatch, logDisplayAppleEvent, false)
        if let previousAppleEventHandler {
            AEInstallSpecialHandler(keyPreDispatch, previousAppleEventHandler, false)
        }
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

    // The SwiftPM probe child doesn't call NSApplication.run(). Deliver only
    // events already queued by AppKit, including the activation notifications.
    func drainEvents(observe: (NSEvent) -> Void = { _ in }) {
        for _ in 0..<64 {
            guard let event = app.nextEvent(matching: .any, until: .distantPast, inMode: .default, dequeue: true) else { break }
            // AppKit raises an exception if subtype is read for an unsupported event type.
            let subtype = switch event.type {
            case .appKitDefined, .systemDefined, .applicationDefined, .leftMouseDown, .leftMouseUp,
                 .rightMouseDown, .rightMouseUp, .otherMouseDown, .otherMouseUp, .mouseMoved,
                 .leftMouseDragged, .rightMouseDragged, .otherMouseDragged:
                String(event.subtype.rawValue)
            default: "n/a"
            }
            logDisplayProbe("\(prefix) drained event type=\(event.type.rawValue) subtype=\(subtype)")
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

@MainActor private final class RunnerDisplayTerminationLog: NSObject, NSApplicationDelegate {
    let prefix: String
    private(set) var requests = 0
    private(set) var notifications = 0

    init(prefix: String) { self.prefix = prefix }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        requests += 1
        let appleEvent = NSAppleEventManager.shared().currentAppleEvent.map {
            " Apple Event class=\(fourCC($0.eventClass)) id=\(fourCC($0.eventID))"
        } ?? ""
        logDisplayProbe("\(prefix) applicationShouldTerminate fired\(appleEvent); returning .terminateCancel")
        return .terminateCancel
    }

    func applicationWillTerminate(_ notification: Notification) {
        notifications += 1
        logDisplayProbe("\(prefix) NSApplication.willTerminateNotification fired")
    }
}

// AppKit may dispatch Apple Events while fetching the next NSEvent. Observe the
// Apple Event Manager's pre-dispatch hook so its class/ID is logged even then.
private let logDisplayAppleEvent: AEEventHandlerUPP = { event, _, _ in
    if let event, let prefix = RunnerDisplayProbe.childPrefix {
        var eventClass: AEEventClass = 0
        var eventID: AEEventID = 0
        AEGetAttributePtr(event, keyEventClassAttr, typeType, nil, &eventClass, MemoryLayout<AEEventClass>.size, nil)
        AEGetAttributePtr(event, keyEventIDAttr, typeType, nil, &eventID, MemoryLayout<AEEventID>.size, nil)
        logDisplayProbe("\(prefix) Apple Event class=\(fourCC(eventClass)) id=\(fourCC(eventID))")
    }
    return OSErr(errAEEventNotHandled) // Continue normal dispatch.
}

private func fourCC(_ code: UInt32) -> String {
    let bytes = withUnsafeBytes(of: code.bigEndian) { Array($0) }
    return bytes.allSatisfy { (32...126).contains($0) }
        ? String(decoding: bytes, as: UTF8.self)
        : "0x\(String(code, radix: 16))"
}
