import Darwin
import Dispatch
import Foundation
import Synchronization
import TestThreadBacktraces

enum TestStackCapture {
    private static let started = Mutex(false)

    // A gate timeout can request an in-process stack when macOS denies it
    // permission to sample. Publish readiness only once the handler exists.
    private static let gateRequests: (any DispatchSourceSignal)? = {
        guard let requests = ProcessInfo.processInfo.environment["OPENUI_SWIFT_TEST_CAPTURE_REQUESTS"] else { return nil }
        let directory = URL(fileURLWithPath: requests, isDirectory: true)
        let pid = getpid()
        signal(SIGUSR2, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGUSR2, queue: .global(qos: .userInitiated))
        source.setEventHandler {
            let partial = directory.appending(path: "stack-\(pid).partial")
            let response = directory.appending(path: "stack-\(pid).txt")
            if writeBacktraces(to: partial, reason: "Swift test gate requested stacks for PID \(pid)") {
                try? FileManager.default.moveItem(at: partial, to: response)
            }
        }
        source.activate()
        try? Data().write(to: directory.appending(path: "ready-\(pid)"))
        return source
    }()

    static func listenForGateRequests() {
        _ = gateRequests
    }

    static func fail(_ message: String) {
        // A blocked main actor can stall several tests at once. Capture it
        // once, and keep it alive until the capture finishes.
        guard started.withLock({ started in
            guard !started else { return false }
            started = true
            return true
        }) else { return }

        let queue = DispatchQueue.global(qos: .userInitiated)
        queue.asyncAfter(deadline: .now() + .seconds(20)) {
            _exit(EXIT_FAILURE)
        }
        // A full console pipe must not keep us from writing the artifact.
        let announced = DispatchSemaphore(value: 0)
        queue.async {
            log(message + "\n")
            announced.signal()
        }
        _ = announced.wait(timeout: .now() + .milliseconds(100))
        capture(message)
        // Reporting a Swift Testing issue alone leaves a blocked runner
        // alive. The gate will report its missing summary as a failure.
        _exit(EXIT_FAILURE)
    }

    private static func capture(_ message: String) {
        let directory = ProcessInfo.processInfo.environment["OPENUI_SWIFT_TEST_STACKS"]
            .map { URL(fileURLWithPath: $0, isDirectory: true) } ?? FileManager.default.temporaryDirectory
        let pid = getpid()
        let base = directory.appending(path: "watchdog-\(pid)-\(UUID())")
        let file = base.appendingPathExtension("sample.txt")
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try (message + "\n").write(to: base.appendingPathExtension("reason.txt"), atomically: false, encoding: .utf8)
            let sample = Process()
            let completion = DispatchSemaphore(value: 0)
            sample.executableURL = URL(fileURLWithPath: "/usr/bin/sample")
            sample.arguments = [String(pid), "5", "-file", file.path]
            sample.standardOutput = FileHandle.nullDevice
            sample.standardError = FileHandle.standardError
            sample.terminationHandler = { _ in completion.signal() }
            var failure: String?
            do {
                try sample.run()
                if completion.wait(timeout: .now() + .seconds(12)) == .timedOut {
                    kill(sample.processIdentifier, SIGKILL)
                    failure = "sample of PID \(pid) timed out after 12s"
                } else if sample.terminationStatus != 0 {
                    failure = "sample of PID \(pid) exited \(sample.terminationStatus)"
                } else if !FileManager.default.fileExists(atPath: file.path) {
                    failure = "sample of PID \(pid) produced no report"
                }
            } catch {
                failure = "could not start sample of PID \(pid): \(error)"
            }
            if let failure {
                let fallback = base.appendingPathExtension("backtrace.txt")
                guard writeBacktraces(to: fallback, reason: failure) else {
                    log("Swift test watchdog: \(failure); could not open \(fallback.path)\n")
                    return
                }
                show(fallback, pid: pid)
            } else {
                show(file, pid: pid)
            }
        } catch {
            log("Swift test watchdog: could not capture PID \(pid): \(error)\n")
        }
    }

    private static func writeBacktraces(to file: URL, reason: String) -> Bool {
        let fd = open(file.path, O_WRONLY | O_CREAT | O_EXCL, S_IRUSR | S_IWUSR)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        try? FileHandle(fileDescriptor: fd).write(contentsOf: Data((reason + "\n").utf8))
        write_test_thread_backtraces(fd)
        return true
    }

    private static func show(_ file: URL, pid: pid_t) {
        if let report = try? String(contentsOf: file, encoding: .utf8) {
            log("Swift test watchdog: stack for PID \(pid) in \(file.path)\n" + report)
        }
    }

    private static func log(_ message: String) {
        try? FileHandle.standardError.write(contentsOf: Data(message.utf8))
    }
}
