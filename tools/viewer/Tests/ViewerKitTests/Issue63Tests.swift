import AppKit
import OpenUINative
import SwiftUI
import Testing
import ViewerKit

@Suite(.serialized) @MainActor struct ViewerIssue63 {
    struct FinalParseFailure: Error {}
    struct FailingFinish: PlaybackSession {
        let session: Session
        func push(_ chunks: [String]) async throws -> Step { try await session.push(chunks) }
        func finish() async throws -> Step { throw FinalParseFailure() }
    }

    @Test func aFailedFinalParseStillClearsTheStreamingGate() async throws {
        let core = Core()
        let library = try await core.library(schema: #"{"$defs":{"TextContent":{"properties":{"text":{}}}}}"#)
        let playback = Playback(core: core, library: library, root: nil, sessionFactory: {
            FailingFinish(session: try await core.session(library: library))
        })
        await playback.load(ReplaySource(program: "root = TextContent(\"ok\")", chunkSize: 8))
        let live = try #require(playback.live)
        await playback.move(to: 1)
        #expect(live.isStreaming)
        await playback.move(to: playback.chunks.count)
        #expect(playback.error != nil)
        #expect(!playback.isStreaming)
        #expect(!live.isStreaming)
    }

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
        func settle() async {
            // Let async main own the run loop; do not start a nested one here.
            try? await Task.sleep(for: .milliseconds(50))
            host.layoutSubtreeIfNeeded()
        }
        func fields(_ view: NSView) -> [NSTextField] {
            ((view as? NSTextField).flatMap { $0.isEditable ? [$0] : nil } ?? []) + view.subviews.flatMap(fields)
        }
        await settle()
        let field = try #require(fields(host).first)
        #expect(field.stringValue == "1")
        #expect(window.makeFirstResponder(field))
        let editor = try #require(field.currentEditor())
        editor.selectAll(nil)
        editor.insertText("25")
        await settle()
        try live.set("$n", to: .number(3))
        await settle()
        #expect(field.stringValue == "25")
        editor.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        await settle()
        #expect(live.value(of: "$n") == .number(25))
        #expect(window.makeFirstResponder(nil))
        try live.set("$n", to: .number(6))
        await settle()
        #expect(field.stringValue == "6")
    }
}
