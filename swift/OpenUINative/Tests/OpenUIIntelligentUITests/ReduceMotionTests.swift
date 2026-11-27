import AppKit
@testable import OpenUIIntelligentUI
import SwiftUI
import SwiftTestSupport
import Testing

// The library's one Reduce Motion value (ReduceMotion.swift). Users get the
// system's setting; only tests, through @testable, pin it.
@Suite @MainActor struct ReduceMotionTests {
    // SwiftUI's accessibilityReduceMotion is read-only, so a test can't put the
    // system in both states. The rule itself is checked in both, and the live
    // value is checked against the system on whichever machine runs it: off
    // here, on in GitHub's macOS runners.
    @Test func withoutAnOverrideTheSeamIsTheSystemValueInBothStates() {
        for system in [false, true] {
            #expect(ReduceMotion.resolve(override: nil, system: system) == system, "system \(system)")
            for pinned in [false, true] {
                #expect(ReduceMotion.resolve(override: pinned, system: system) == pinned,
                        "system \(system), pinned \(pinned)")
            }
        }
        var environment = EnvironmentValues()
        #expect(environment.reduceMotionOverride == nil)
        #expect(environment.reducesMotion == environment.accessibilityReduceMotion)
        environment.reduceMotionOverride = !environment.accessibilityReduceMotion
        #expect(environment.reducesMotion != environment.accessibilityReduceMotion)
    }

    @Test(NativeTestWatchdog()) func aMountedViewReadsTheSystemSettingUnlessATestPinsIt() async throws {
        let system = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        for pinned in [nil, false, true] as [Bool?] {
            let read = try await mountedValues(pinned: pinned)
            // The environment SwiftUI hands the library carries the system's setting.
            #expect(read.accessibility == system, "pinned \(String(describing: pinned))")
            #expect(read.seam == (pinned ?? system),
                    "pinned \(String(describing: pinned)): seam \(read.seam), system \(system)")
        }
    }

    @MainActor private final class Values {
        var accessibility: Bool?
        var seam: Bool?
    }

    private struct Probe: View {
        let values: Values
        @Environment(\.accessibilityReduceMotion) private var accessibility
        @Environment(\.reducesMotion) private var seam

        var body: some View {
            values.accessibility = accessibility
            values.seam = seam
            return Color.clear.frame(width: 10, height: 10)
        }
    }

    private func mountedValues(pinned: Bool?) async throws -> (accessibility: Bool, seam: Bool) {
        NativeTestWatchdog.waitingFor("AppKit to mount the Reduce Motion probe")
        let values = Values()
        let host = NSHostingView(rootView: Probe(values: values).environment(\.reduceMotionOverride, pinned))
        let window = NSWindow(contentRect: CGRect(x: 60, y: 60, width: 40, height: 40),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.closeForTest() }
        window.orderFrontRegardless()
        await layoutOnMainQueue(host, waitingFor: "AppKit to lay out the Reduce Motion probe")
        guard let accessibility = values.accessibility, let seam = values.seam else {
            throw CollectionInputFailure(description: "the Reduce Motion probe never drew")
        }
        return (accessibility, seam)
    }
}

// Every Reduce Motion branch reads the seam, so a test can pin it. A branch
// that read the system directly would follow the machine again: on GitHub's
// runners, only the reduced path.
@Test func onlyTheSeamReadsTheSystemReduceMotionSetting() throws {
    var repository = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { repository.deleteLastPathComponent() }
    let seam = "swift/OpenUINative/Sources/OpenUIIntelligentUI/ReduceMotion.swift"
    let readers = ["accessibilityReduceMotion", "accessibilityDisplayShouldReduceMotion", "isReduceMotionEnabled"]
    var scanned = 0
    var offenders: [String] = []
    for root in ["swift/OpenUINative/Sources", "tools/chat/Sources"] {
        let directory = repository.appending(path: root)
        // An enumerator over a missing folder is non-nil and empty, so check
        // the folder and count what each one yields.
        var isDirectory: ObjCBool = false
        let exists = FileManager.default.fileExists(atPath: directory.path, isDirectory: &isDirectory)
        #expect(exists && isDirectory.boolValue, "no \(root) under \(repository.path)")
        let files = try #require(FileManager.default.enumerator(at: directory, includingPropertiesForKeys: nil),
                                 "no \(root) under \(repository.path)")
        var inRoot = 0
        for case let file as URL in files where file.pathExtension == "swift" {
            let relative = String(file.standardizedFileURL.path.dropFirst(repository.standardizedFileURL.path.count + 1))
            let source = try String(contentsOf: file, encoding: .utf8)
            scanned += 1
            inRoot += 1
            if relative != seam, readers.contains(where: { source.contains($0) }) { offenders.append(relative) }
        }
        #expect(inRoot > 0, "no Swift files under \(root)")
    }
    #expect(scanned > 20, "scanned only \(scanned) Swift files")
    #expect(offenders == [], "read Reduce Motion outside \(seam)")
    let seamSource = try String(contentsOf: repository.appending(path: seam), encoding: .utf8)
    #expect(seamSource.contains("accessibilityReduceMotion"))
}
