import Foundation
import Testing

@Suite struct RenderingCompatibility {
    /// Compare real SwiftUI rendering in isolated package copies.
    @Test func everyCorpusAndBenchmarkProgramPreservesMainDrawing() async throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let output = root.appendingPathComponent("target/swift-71-r3-differential-suite")
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let log = output.appendingPathComponent("driver.log")
        #expect(FileManager.default.createFile(atPath: log.path, contents: nil))
        let file = try FileHandle(forWritingTo: log)
        defer { try? file.close() }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["python3", root.appendingPathComponent("tools/viewer/Checks/compare-native-rendering.py").path,
                             "--output", output.path]
        process.currentDirectoryURL = root
        process.standardOutput = file
        process.standardError = file
        try process.run()
        defer { if process.isRunning { process.terminate() } }
        let clock = ContinuousClock()
        // The driver allows four minutes for each of its two builds and probes.
        let deadline = clock.now + .seconds(600)
        while process.isRunning && clock.now < deadline {
            try await clock.sleep(until: min(deadline, clock.now + .milliseconds(100)))
        }
        // Full build/probe output is kept in target; bound assertion diagnostics.
        let diagnostic = try String(contentsOf: log, encoding: .utf8).suffix(4_096)
        try #require(!process.isRunning, "Rendering differential PID \(process.processIdentifier) still running at the deadline. \(diagnostic)")
        #expect(process.terminationStatus == 0, "Rendering differential failed: \(diagnostic)")
    }
}
