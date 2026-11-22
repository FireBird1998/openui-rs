import Foundation
import OpenUINative
import Testing

struct MessageTests {
    @Test func writesOrderedAttributesContextAndEnd() async throws {
        let raw = try await Core().buildMessage(content: "Save", context: .string("raw context"),
            attributes: [("z", "a b"), ("a", "+")], end: true)
        #expect(raw == "]]>openui:content?z=a+b&a=%2B\nSave\n]]>openui:context\nraw context\n]]>openui:end")
    }

    @Test func readsWinningSectionAndHoldsStreamingMarker() async throws {
        let core = Core()
        let raw = "discarded]]>openui:content?z=a+b&a=%2B\nSave\n]]>openui:context\n[0,false]\n]]>openui:end"
        let complete = try await core.parseMessage(raw)
        #expect(complete.content == "Save")
        #expect(complete.context == .array([.number(0), .bool(false)]))
        #expect(complete.attributes.map(\.key) == ["z", "a"])
        #expect(complete.attributes.map(\.value) == ["a b", "+"])
        #expect(complete.end)
        let streaming = try await core.parseMessage(raw, streaming: true)
        #expect(!streaming.end)
        #expect(streaming.context == complete.context)
    }

    @Test func readsMcpValuesAndToolReportedErrors() async throws {
        let core = Core()
        #expect(try await core.extractMcpToolResult(#"{"structuredContent":false,"content":[{"type":"text","text":"fallback"}]}"#)
            == .value(.bool(false)))
        #expect(try await core.extractMcpToolResult(#"{"content":[{"type":"text","text":"[1"},{"type":"text","text":"]"}]}"#)
            == .value(.array([.number(1)])))
        #expect(try await core.extractMcpToolResult("{}") == .value(.null))
        #expect(try await core.extractMcpToolResult(#"{"isError":true,"content":[{"type":"text","text":"first"},{"type":"text","text":"second"}]}"#)
            == .toolReported("first\nsecond"))
    }

    @Test(arguments: ["label", "form", "context", "markers", "emptyLabel", "emptyForm", "formAndContext"])
    func actionMessageMatchesRecordedUTF8(_ name: String) async throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let url = root.appendingPathComponent("conformance/golden/action-messages.json")
        let cases = try JSONValue.parse(String(contentsOf: url, encoding: .utf8))
        let event = try #require(cases[name])
        let label = try #require(event["humanFriendlyMessage"]?.string)
        let expected = try #require(event["message"]?.string)
        let message = try await Core().buildActionUserMessage(label: label, formState: event["formState"],
                                                               context: event["params"]?["context"])
        #expect(message.utf8.elementsEqual(expected.utf8))
    }

    @MainActor @Test func formSubmitBuildsTheNextTurnFromItsHostEvent() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        root = Form("booking", [], Buttons([Button("Submit", @ToAssistant("Save", "checkout"))]))
        """) { events.append($0) }
        try live.setField(form: "booking", name: "seat", to: .string("3A"))
        let button = try #require(element(live, at: "/buttons/buttons/0"))
        try live.run("action", of: button, message: "Submit", form: "booking")
        let event = try #require(events.first)
        let raw = try await Core().buildActionUserMessage(event)
        #expect(raw == """
        ]]>openui:content
        Save
        ]]>openui:context
        ["User clicked: Save",{"booking":{"seat":{"value":"3A"}}},"checkout"]
        """)
    }

    @Test func messageRefusalsKeepTheirCoreKinds() async {
        let core = Core()
        let invalid = await #expect(throws: OpenUIError.self) {
            try await core.buildMessage(content: "", attributes: [("x", "\u{10F800}")])
        }
        #expect(invalid?.kind == .invalidAttribute)
        let nested = String(repeating: "[", count: 801) + "0" + String(repeating: "]", count: 801)
        let deep = await #expect(throws: OpenUIError.self) {
            try await core.parseMessage("]]>openui:context\n" + nested)
        }
        #expect(deep == OpenUIError(.limitExceeded, "message JSON exceeds the depth limit"))
        let large = await #expect(throws: OpenUIError.self) {
            try await core.buildActionUserMessage(label: String(repeating: "a", count: 1 << 19))
        }
        #expect(large?.kind == .limitExceeded)
    }

    @Test func mcpRefusalsKeepTheirCoreKinds() async {
        let core = Core()
        let malformed = await #expect(throws: OpenUIError.self) { try await core.extractMcpToolResult("{") }
        #expect(malformed?.kind == .invalidJSON)
        let shape = await #expect(throws: OpenUIError.self) { try await core.extractMcpToolResult(#"{"content":1}"#) }
        #expect(shape == OpenUIError(.evaluation, "content.filter is not a function"))
        let deep = await #expect(throws: OpenUIError.self) {
            try await core.extractMcpToolResult(String(repeating: "[", count: 801) + String(repeating: "]", count: 801))
        }
        #expect(deep?.kind == .limitExceeded)
    }
}
