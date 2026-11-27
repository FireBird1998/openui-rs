import Darwin
import Dispatch
import Foundation
import Synchronization
import TestThreadBacktraces

enum TestStackCapture {
    private static let captureStarted = Mutex(false)

    static func fail(_ name: String, after seconds: Int, waitingFor operation: String) {
        // A blocked main actor can stall several test cases at once.
        guard captureStarted.withLock({ started in
            guard !started else { return false }
            started = true
            return true
        }) else { return }

        let queue = DispatchQueue.global(qos: .userInitiated)
        queue.asyncAfter(deadline: .now() + .seconds(10)) { _exit(EXIT_FAILURE) }
        let files = FileManager.default
        let directory = ProcessInfo.processInfo.environment["OPENUI_SWIFT_TEST_STACKS"]
            .map { URL(fileURLWithPath: $0, isDirectory: true) }
            ?? files.temporaryDirectory.appending(path: "swift-test-stacks", directoryHint: .isDirectory)
        let stem = "watchdog-\(getpid())-\(UUID())"
        let heading = "Test \(name)\n"
        let diagnostics = DispatchGroup()
        queue.async(group: diagnostics) {
            log("Swift test watchdog: \(name): timed out after \(seconds)s waiting for \(operation)\n")
        }
        queue.async(group: diagnostics) {
            do {
                let graph = try sample(beside: directory)
                let report = heading + graph
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                let path = directory.appending(path: "\(stem).sample.txt")
                try report.write(to: path, atomically: true, encoding: .utf8)
                log("Swift test watchdog: stack in \(path.path)\n" + report)
            } catch {
                log("Swift test watchdog: /usr/bin/sample did not produce a stack: \(error)\n")
            }
        }
        do {
            try files.createDirectory(at: directory, withIntermediateDirectories: true)
            let path = directory.appending(path: "\(stem).backtrace.txt")
            try writeBacktraces(heading, to: path)
            let report = try String(contentsOf: path, encoding: .utf8)
            log("Swift test watchdog: stack in \(path.path)\n" + report)
        } catch {
            log("Swift test watchdog: could not write in-process backtraces: \(error)\n")
        }
        diagnostics.wait()
        _exit(EXIT_FAILURE)
    }

    private static func writeBacktraces(_ heading: String, to path: URL) throws {
        let fd = open(path.path, O_WRONLY | O_CREAT | O_EXCL, S_IRUSR | S_IWUSR)
        guard fd >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        let file = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { try? file.close() }
        try file.write(contentsOf: Data(heading.utf8))
        write_test_thread_backtraces(fd)
    }

    private static func sample(beside directory: URL) throws -> String {
        let files = FileManager.default
        // Keep the unfiltered sampler output out of the artifact directory.
        let scratch = directory.deletingLastPathComponent().appending(path: "swift-test-sample-\(UUID())")
        try files.createDirectory(at: scratch, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        defer { try? files.removeItem(at: scratch) }
        let raw = scratch.appending(path: "report.txt")
        let sampler = Process()
        sampler.executableURL = URL(fileURLWithPath: "/usr/bin/sample")
        sampler.arguments = [String(getpid()), "5", "-file", raw.path]
        sampler.standardOutput = FileHandle.nullDevice
        sampler.standardError = FileHandle.nullDevice
        let finished = DispatchSemaphore(value: 0)
        sampler.terminationHandler = { _ in finished.signal() }
        try sampler.run()
        guard finished.wait(timeout: .now() + .seconds(7)) == .success else {
            kill(sampler.processIdentifier, SIGKILL)
            throw POSIXError(.ETIMEDOUT)
        }
        guard sampler.terminationStatus == 0 else { throw POSIXError(.EIO) }

        let file = try FileHandle(forReadingFrom: raw)
        defer { try? file.close() }
        let output = String(decoding: try file.read(upToCount: 2 * 1024 * 1024) ?? Data(), as: UTF8.self)
        guard let start = output.range(of: "Call graph:\n")?.upperBound else { throw POSIXError(.EIO) }
        let graph = output[start...].components(separatedBy: "\nTotal number in stack")[0]
            .components(separatedBy: "\nBinary Images:")[0]
        guard !graph.isEmpty else { throw POSIXError(.EIO) }
        return "Call graph:\n" + graph
    }

    private static func log(_ message: String) {
        try? FileHandle.standardError.write(contentsOf: Data(message.utf8))
    }
}
