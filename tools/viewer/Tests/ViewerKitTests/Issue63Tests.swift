import AppKit
import OpenUINative
import SwiftUI
import Testing
import ViewerKit

extension ViewerWindowTests {
@Suite @MainActor struct ViewerIssue63 {
    @Test func aMountedStoreDraftKeepsEditsAndCommitsTheCurrentValue() async throws {
        let core = Core()
        let library = try await core.library(schema: #"{"$defs":{"TextContent":{"properties":{"text":{}}}}}"#)
        let playback = Playback(core: core, library: library, root: nil)
        await playback.load(ReplaySource(program: "$n = 1\nroot = TextContent($n)", chunkSize: 100))
        let live = try #require(playback.live)
        let host = NSHostingView(rootView: StorePanel(playback: playback))
        let window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: 500, height: 500), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close() }
        func fields(_ view: NSView) -> [NSTextField] {
            ((view as? NSTextField).flatMap { $0.isEditable ? [$0] : nil } ?? []) + view.subviews.flatMap(fields)
        }
        await layoutOnMainQueue(host)
        let field = try #require(fields(host).first)
        #expect(field.stringValue == "1")
        #expect(window.makeFirstResponder(field))
        let editor = try #require(field.currentEditor())
        editor.selectAll(nil)
        editor.insertText("25")
        await layoutOnMainQueue(host)
        try live.set("$n", to: .number(3))
        await layoutOnMainQueue(host)
        #expect(field.stringValue == "25")
        editor.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        await layoutOnMainQueue(host)
        #expect(live.value(of: "$n") == .number(25))
        #expect(window.makeFirstResponder(nil))
        try live.set("$n", to: .number(6))
        await layoutOnMainQueue(host)
        #expect(field.stringValue == "6")
    }
}
}
