import AppKit
import Foundation
import OpenUINative
import OpenUIFFI
import SwiftUI
import Testing

extension NativeUITests {
@Suite @MainActor struct Issue63 {
    @Test func aSecondClickChecksTheCurrentFormBeforeAnyRedraw() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        $required = false
        root = Form("f", [Input("x", null, {required: $required})], Buttons([Button("Require", @Set($required, true)), Button("Send", @ToAssistant("sent"))]))
        """) { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        let require = try window.clickEvents("/buttons/buttons/0")
        let send = try window.clickEvents("/buttons/buttons/1")
        for event in require + send { window.window.sendEvent(event) }
        #expect(live.value(of: "$required") == .bool(true))
        #expect(events.isEmpty)
        #expect(live.failure == nil)
    }

    @Test func identicalButtonsAndMarkersFromAnEarlierShowAreRefused() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library, bindings: [("Input", "value")])
        let step = try await core.parseOneShot("$n = 0\nroot = Stack([Button(\"Go\", @Set($n, $n + 1)), Input(\"n\", $n)])", library: library)
        try live.show(step)
        let old = try #require(element(live, at: "/children/0"))
        let input = try #require(element(live, at: "/children/1"))
        guard case let .binding(marker)? = input.props.first(where: { $0.name == "value" })?.value else { Issue.record("missing marker"); return }
        try live.show(step)
        let version = live.version
        #expect(throws: OpenUIError.self) { try live.run("action", of: old, message: "Go") }
        #expect(throws: OpenUIError.self) { try live.write(marker, .number(99)) }
        #expect(throws: OpenUIError.self) { try live.enter(.number(99), into: input) }
        #expect(live.version == version)
        #expect(live.value(of: "$n") == .number(0))
    }

    @Test func snapshotFieldReadsKeepObjectsWithoutAValueKey() async throws {
        let live = try await live("root = Stack([])")
        for value in [JSONValue.object([]), .object([("componentType", .string("data"))]), .object([("value", .null)]), .object([("value", .number(7)), ("componentType", .string("Input"))])] {
            try live.set("f", to: .object([("x", value)]))
            try live.set("x", to: value)
            let expected = value["value"] ?? value
            #expect(live.field(form: "f", name: "x") == expected)
            #expect(live.field(form: nil, name: "x") == expected)
        }
    }

    @Test func undefinedFieldWritesAndLookalikeDataMatchTheRuntimeGetter() async throws {
        let live = try await live("root = Stack([])")
        let session = try RuntimeSession(library: ComponentLibrary(schema: liveSchema), promptSpec: nil)
        defer { session.dispose() }
        for form in [nil, "f"] as [String?] {
            for type in [nil, "Input"] as [String?] {
                _ = try session.setField(form: form, componentType: type, name: "x", json: nil, save: true)
                try live.setField(form: form, componentType: type, name: "x", to: nil)
                let exported = try session.getField(form: form, name: "x")
                let expected = try exported.map(JSONValue.parse)
                #expect(live.field(form: form, name: "x") == expected)
            }
        }
    }

    @Test(.requiresDisplay("selectingTheAlreadySelectedValueDoesNotApplyItsTransform()")) func selectingTheAlreadySelectedValueDoesNotApplyItsTransform() async throws {
        let live = try await live("""
        $v = "a"
        root = Select("s", [SelectItem("a", "A"), SelectItem("b", "B")], $v = $value + "!")
        """, bindings: [("Select", "value")])
        let window = await Window(live)
        defer { window.close() }
        let version = live.version
        try await window.pick("A", at: "/")
        #expect(live.value(of: "$v") == .string("a"))
        #expect(live.version == version)
        #expect(live.pristineNames.contains("$v"))
        let reopened = await Window(live)
        defer { reopened.close() }
        try await reopened.pick("B", at: "/")
        #expect(live.value(of: "$v") == .string("b!"))
    }

    @Test func enteringASelectsCurrentValueDoesNotApplyItsTransform() async throws {
        let live = try await live("""
        $v = "a"
        root = Select("s", [SelectItem("a", "A"), SelectItem("b", "B")], $v = $value + "!")
        """, bindings: [("Select", "value")])
        let select = try #require(live.root)
        let version = live.version
        try live.enter(.string("a"), into: select)
        #expect(live.value(of: "$v") == .string("a"))
        #expect(live.version == version)
        #expect(live.pristineNames.contains("$v"))
        try live.enter(.string("b"), into: select)
        #expect(live.value(of: "$v") == .string("b!"))
        live.isStreaming = true
        let streamingVersion = live.version
        #expect(throws: OpenUIError.self) { try live.enter(.string("c"), into: select) }
        #expect(live.version == streamingVersion)
        #expect(live.value(of: "$v") == .string("b!"))
    }

    @Test func tableActionCellsValidateTheirCurrentForm() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        root = Form("f", [Input("x", null, {required: true}), Table([Col("Do", "action")], [["Open"]])], Buttons([]))
        """) { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        let version = live.version
        let point: (CGRect) -> CGPoint = { CGPoint(x: $0.minX + 8, y: $0.maxY - 6) }
        try await window.click("/fields/1", at: point)
        #expect(events.isEmpty)
        #expect(live.version == version)
        try live.setField(form: "f", name: "x", to: .string("yes"))
        await window.settle()
        try await window.click("/fields/1", at: point)
        #expect(events.map(\.humanFriendlyMessage) == ["Open"])
    }

    static let extendedSchema: String = {
        var base = liveSchema
        let added = ["TextArea", "RadioGroup", "CheckBoxGroup", "Slider", "DatePicker", "Chips", "OptionCards", "CustomField"].map {
            "\"\($0)\": {\"properties\": {\"name\": {}, \"value\": {}, \"rules\": {}, \"defaultValue\": {}, \"type\": {}}}"
        }.joined(separator: ",")
        base = base.replacingOccurrences(of: "\"$defs\": {", with: "\"$defs\": {" + added + ",\"Tabs\": {\"properties\": {\"items\": {}}},\"TabItem\": {\"properties\": {\"trigger\": {}, \"content\": {}}},")
        return base
    }()

    func extended(_ source: String, events: @escaping @MainActor (HostEvent) -> Void) async throws -> LiveProgram {
        let core = Core()
        let library = try await core.library(schema: Self.extendedSchema)
        let live = try LiveProgram(library: library, onHostEvent: events)
        try live.show(try await core.parseOneShot(source, library: library))
        return live
    }

    @Test(arguments: ["TextArea", "RadioGroup", "CheckBoxGroup", "Slider", "DatePicker", "Chips", "OptionCards"])
    func everyUpstreamFieldParticipatesInValidation(_ type: String) async throws {
        var events: [HostEvent] = []
        let live = try await extended("root = Form(\"f\", [\(type)(\"x\", null, {required: true})], Buttons([Button(\"Send\")]))") { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        try live.setField(form: "f", name: "x", to: type == "Chips" ? .array([.string("yes")]) : .string("yes"))
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
    }

    @Test(arguments: ["Chips", "OptionCards"])
    func selectionDefaultsAndStoredShapesFollowUpstream(_ type: String) async throws {
        for (mode, initial, stored, allows) in [
            ("single", "[\"a\", \"b\"]", JSONValue.string(""), false),
            ("multiple", "[1, \"a\"]", .array([]), false),
            ("single", "\"a\"", .array([.string("a")]), false),
            ("multiple", "\"a\"", .string("a"), false),
            ("single", "\"a\"", .string("b"), true),
            ("multiple", "\"a\"", .array([.string("b")]), true),
        ] {
            var events: [HostEvent] = []
            let live = try await extended("root = Form(\"f\", [\(type)(\"x\", null, {required: true}, \(initial), \"\(mode)\")], Buttons([Button(\"Send\")]))") { events.append($0) }
            let window = await Window(live)
            defer { window.close() }
            try await window.click("/buttons/buttons/0")
            #expect(events.count == 1)
            try live.setField(form: "f", name: "x", to: stored)
            await window.settle()
            try await window.click("/buttons/buttons/0")
            #expect(events.count == (allows ? 2 : 1))
        }
    }

    @Test func aggregateValuePropsAreValidatedWithoutDiscardingTheirData() async throws {
        var events: [HostEvent] = []
        let live = try await extended("root = Form(\"f\", [CheckBoxGroup(\"x\", {a: true}, {required: true}), Slider(\"s\", [4], {min: 3})], Buttons([Button(\"Send\")]))") { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
        try live.setField(form: "f", name: "x", to: .object([("a", .bool(false))]))
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
    }

    @Test func aCustomComponentsRegisteredValueIsValidated() async throws {
        var events: [HostEvent] = []
        let live = try await extended("$custom = \"\"\nroot = Form(\"f\", [CustomField(\"x\", null, {required: true})], Buttons([Button(\"Send\")]))") { events.append($0) }
        live.registerValidationComponent("CustomField") { _, live, _ in live.value(of: "$custom") }
        let window = await Window(live, component: { element, _ in element.typeName == "CustomField" ? AnyView(Text("App field")) : nil })
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        try live.set("$custom", to: .string("ready"))
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
        live.registerValidationComponent("CustomField") { _, _, _ in .string("") }
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
        live.registerValidationComponent("CustomField", value: nil)
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 2)
    }

    @Test func onlyTheVisibleTabIsValidatedIncludingAfterSwitching() async throws {
        var events: [HostEvent] = []
        let live = try await extended("root = Form(\"f\", [Tabs([TabItem(\"First\", [Input(\"a\", \"ok\", {required: true})]), TabItem(\"Second\", [Input(\"b\", null, {required: true})])])], Buttons([Button(\"Send\")]))") { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
        try await window.click("/fields/0", at: { CGPoint(x: $0.minX + 95, y: $0.minY + 12) })
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
        try live.setField(form: "f", name: "b", to: .string("ok"))
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 2)
    }

    @Test func anAppsTabsValidateEveryContentWhenNoNativeSelectionIsRecorded() async throws {
        var events: [HostEvent] = []
        let live = try await extended("root = Form(\"f\", [Tabs([TabItem(\"First\", [Input(\"a\", \"ok\", {required: true})]), TabItem(\"Second\", [Input(\"b\", null, {required: true})])])], Buttons([Button(\"Send\")]))") { events.append($0) }
        let window = await Window(live, component: { element, _ in
            guard element.typeName == "Tabs" else { return nil }
            return AnyView(AllTabContents(element: element))
        })
        defer { window.close() }
        #expect(window.fields.map(\.stringValue) == ["ok", ""])
        let version = live.version
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        #expect(live.version == version)
        try live.setField(form: "f", name: "b", to: .string("ok"))
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
    }

    @Test(arguments: ["Input", "Select"], [#"["a"]"#, "{a:1}"])
    func emptyNativeFieldsCannotUseInvisibleCompositeDefaults(_ type: String, _ initial: String) async throws {
        var events: [HostEvent] = []
        let items = type == "Select" ? #"[SelectItem("a", "A")], "# : ""
        let live = try await live("root = Form(\"f\", [\(type)(\"x\", \(items)\(initial), {required: true})], Buttons([Button(\"Send\")]))") { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        if type == "Input" { #expect(window.fields.map(\.stringValue) == [""]) }
        let version = live.version
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        #expect(live.version == version)
        try live.setField(form: "f", name: "x", to: .string("a"))
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
    }
}

}

/// An app draws both tabs through the public component hook and child view API.
private struct AllTabContents: View {
    let element: Element

    var children: [Element] {
        guard case let .list(items)? = element.props.first(where: { $0.name == "items" })?.value else { return [] }
        return items.flatMap { item -> [Element] in
            guard case let .element(tab) = item,
                  case let .list(content)? = tab.props.first(where: { $0.name == "content" })?.value else { return [] }
            return content.compactMap { if case let .element(child) = $0 { child } else { nil } }
        }
    }

    var body: some View {
        VStack {
            ForEach(Array(children.enumerated()), id: \.offset) { _, child in
                ElementView(element: child)
            }
        }
    }
}

@Suite struct BoundedJSONReading {
    @Test func parsingAndReleasingDeepJSONOnA512KBStackIsBounded() async {
        await withCheckedContinuation { continuation in
            let thread = Thread {
                for depth in [0, 250, 386, 511, 512, 4000, 100000] {
                    for object in [false, true] {
                        let text = String(repeating: object ? "{\"x\":" : "[", count: depth) + "0" + String(repeating: object ? "}" : "]", count: depth)
                        do {
                            _ = try JSONValue.parse(text) // Release an entire nested value on this stack too.
                            var value = try JSONValue.parse(text)
                            #expect(depth < 512)
                            for _ in 0..<depth { value = object ? value["x"] ?? .null : value.array?.first ?? .null }
                            #expect(value == .number(0))
                        } catch {
                            #expect(depth >= 512)
                            #expect((error as? JSONError)?.reason == "nested too deeply")
                        }
                    }
                }
                continuation.resume()
            }
            thread.stackSize = 512 * 1024
            thread.start()
        }
    }
}
