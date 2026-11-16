// When the redraw after an action fails, the action's host events are still
// delivered, and the failure is thrown after them. Nothing a program does
// makes the redraw fail with the default limits, so the session is disposed
// underneath the live program between the action and its redraw: the real
// output of a real run, then a real failure.
import OpenUIFFI
@testable import OpenUINative
import Testing

extension NativeUITests {
@Suite @MainActor struct ARedrawThatFails {
    @Test func stillDeliversTheActionsHostEvents() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        $n = 0
        root = Stack([TextContent("n: " + $n), Button("Go", Action([@Set($n, 1), @ToAssistant("went")]))])
        """) { events.append($0) }
        let button = try #require(element(live, at: "/children/1"))
        let tree = try live.session.evaluate()
        let action = try live.session.actionAt(evaluation: tree.evaluation, path: ["props", "children", "1", "props", "action"])
        let output = try live.session.run(action: action, message: "Go", form: nil)
        live.session.dispose()
        #expect(throws: OpenUiError.self) { try live.finish(output) }
        #expect(events.map(\.type) == ["continue_conversation"])
        #expect(events.first?.humanFriendlyMessage == "went")
        #expect(live.failure != nil)
        #expect(throws: (any Error).self) { try live.run("action", of: button, message: "Go") }
    }
}

}
