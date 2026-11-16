import Foundation
import Testing

@Suite struct RenderingCompatibility {
    /// Compare real SwiftUI rendering in isolated package copies. Await process
    /// termination asynchronously; never nest a main run loop or block its UI.
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
        let status: Int32 = try await withCheckedThrowingContinuation { continuation in
            process.terminationHandler = { continuation.resume(returning: $0.terminationStatus) }
            do { try process.run() }
            catch { continuation.resume(throwing: error) }
        }
        // Full build/probe output is kept in target; bound assertion diagnostics.
        let diagnostic = try String(contentsOf: log, encoding: .utf8).suffix(4_096)
        #expect(status == 0, "Rendering differential failed: \(diagnostic)")
    }
}
