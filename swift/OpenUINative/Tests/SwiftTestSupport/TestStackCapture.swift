import Darwin
import Dispatch
import Foundation
import Synchronization
import TestThreadBacktraces

/// The helper passes stacks through files because Swift Testing can suppress
/// its observed stderr on an exit mismatch. The wait label stays private.
public struct ExitTestDiagnostics: Codable, Sendable {
    let stacks: URL
    let failure: URL
    private let childDeadline: TimeInterval

    public init(in scratch: URL, childSeconds: TimeInterval? = nil) {
        let id = "exit-test-\(UUID())"
        stacks = TestStackCapture.directory(fallback: scratch).appending(path: id)
        failure = scratch.appending(path: id + ".console")
        childDeadline = ProcessInfo.processInfo.systemUptime
            + (childSeconds ?? TimeInterval(SwiftTestLimits.shared.exitChildSeconds))
    }

    var childTimeRemaining: Duration { .seconds(childDeadline - ProcessInfo.processInfo.systemUptime) }

    public var parentAllowance: Duration {
        // Child and parent count from the same launch deadline, even if Swift
        // Testing takes time to start the child or its checkpoints keep coming.
        let limits = SwiftTestLimits.shared
        return childTimeRemaining + .seconds(limits.captureSeconds + limits.schedulingSeconds)
    }

    public func show() {
        TestStackCapture.showExitFailure(self)
    }
}

enum TestStackCapture {
    private static let started = Mutex(false)
    private static let active = Mutex<[UUID: String]>([:])
    private static let lastProgress = Mutex<ContinuousClock.Instant?>(nil)
    private static let requests = ProcessInfo.processInfo.environment["OPENUI_SWIFT_TEST_CAPTURE_REQUESTS"]
        .map { URL(fileURLWithPath: $0, isDirectory: true) }
    private static let activeMarker = requests?.appending(path: "active-\(getpid())")

    // Publish readiness only once the in-process handler exists.
    private static let gateRequests: (any DispatchSourceSignal)? = {
        guard let requests else { return nil }
        let pid = getpid()
        signal(SIGUSR2, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGUSR2, queue: .global(qos: .userInitiated))
        source.setEventHandler {
            let partial = requests.appending(path: "stack-\(pid).partial")
            let response = requests.appending(path: "stack-\(pid).txt")
            if writeBacktraces(to: partial) {
                try? FileManager.default.moveItem(at: partial, to: response)
            }
        }
        source.activate()
        try? Data().write(to: requests.appending(path: "ready-\(pid)"))
        return source
    }()

    static func register(_ test: String) -> UUID {
        let id = UUID()
        _ = gateRequests
        active.withLock { tests in
            if tests.isEmpty, let activeMarker { try? Data().write(to: activeMarker) }
            tests[id] = test
        }
        return id
    }

    static func unregister(_ id: UUID) {
        active.withLock { tests in
            tests.removeValue(forKey: id)
            if tests.isEmpty, let activeMarker { try? FileManager.default.removeItem(at: activeMarker) }
        }
    }

    static func reportProgress() {
        guard let requests else { return }
        let now = ContinuousClock.now
        let publish = lastProgress.withLock { last in
            if let last, last.duration(to: now) < .seconds(1) { return false }
            last = now
            return true
        }
        if publish {
            let checkpoint = Data(UUID().uuidString.utf8)
            try? checkpoint.write(to: requests.appending(path: "progress"), options: .atomic)
        }
    }

    static func directory(fallback: URL = FileManager.default.temporaryDirectory) -> URL {
        ProcessInfo.processInfo.environment["OPENUI_SWIFT_TEST_STACKS"]
            .map { URL(fileURLWithPath: $0, isDirectory: true) } ?? fallback
    }

    static func fail(_ name: String, waitingFor operation: String, elapsed: Duration, diagnostics: ExitTestDiagnostics?) {
        // A blocked main actor can stall several cases at once. Capture it once.
        guard started.withLock({ started in
            guard !started else { return false }
            started = true
            return true
        }) else { return }

        let queue = DispatchQueue.global(qos: .userInitiated)
        queue.asyncAfter(deadline: .now() + .seconds(SwiftTestLimits.shared.captureSeconds)) { _exit(EXIT_FAILURE) }
        let directory = diagnostics?.stacks ?? directory()
        let report = directory.appending(path: "watchdog-\(getpid())-\(UUID()).backtrace.txt")
        let message = "Swift test watchdog: \(name): timed out after \(elapsed) waiting for \(operation)\n"
        if let diagnostics {
            try? Data(message.utf8).write(to: diagnostics.failure, options: .atomic)
        }
        // Console backpressure cannot stop the backtrace from being written.
        let announced = DispatchSemaphore(value: 0)
        queue.async {
            log(message)
            announced.signal()
        }
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            if writeBacktraces(to: report) {
                show(report)
            } else {
                log("Swift test watchdog: could not write stacks in \(directory.path)\n")
            }
        } catch {
            log("Swift test watchdog: could not open stack directory: \(error)\n")
        }
        _ = announced.wait(timeout: .now() + .milliseconds(100))
        _exit(EXIT_FAILURE)
    }

    private static func writeBacktraces(to file: URL) -> Bool {
        let fd = open(file.path, O_WRONLY | O_CREAT | O_EXCL, S_IRUSR | S_IWUSR)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        let names = active.withLock { Set($0.values).sorted() }
        try? FileHandle(fileDescriptor: fd).write(contentsOf: Data(names.map { "Test \($0)\n" }.joined().utf8))
        write_test_thread_backtraces(fd)
        return true
    }

    static func showExitFailure(_ diagnostics: ExitTestDiagnostics) {
        if let file = try? FileHandle(forReadingFrom: diagnostics.failure) {
            defer { try? file.close() }
            if let message = try? file.read(upToCount: 4096) {
                log(String(decoding: message, as: UTF8.self))
            }
        }
        guard let reports = try? FileManager.default.contentsOfDirectory(at: diagnostics.stacks,
                                                                        includingPropertiesForKeys: nil) else { return }
        for report in reports where report.lastPathComponent.hasSuffix(".backtrace.txt") {
            show(report)
        }
    }

    private static func show(_ file: URL) {
        if let report = try? String(contentsOf: file, encoding: .utf8) {
            log("Swift test watchdog: stack in \(file.path)\n" + report)
        }
    }

    private static func log(_ message: String) {
        try? FileHandle.standardError.write(contentsOf: Data(message.utf8))
    }
}
