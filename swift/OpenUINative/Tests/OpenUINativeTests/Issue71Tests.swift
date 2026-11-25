import AppKit
import Foundation
import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

@MainActor private func reactLibrary(_ core: Core) async throws -> (Library, String) {
    var directory = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { directory.deleteLastPathComponent() }
    let schema = try String(contentsOf: directory.appendingPathComponent("conformance/imported/schemas/react-ui.json"), encoding: .utf8)
    let spec = try String(contentsOf: directory.appendingPathComponent("conformance/prompts/react-ui-schema.json"), encoding: .utf8)
    NativeTestWatchdog.waitingFor("Core to load the React UI schema")
    return (try await core.library(schema: schema), spec)
}

extension NativeUITests {
@Suite @MainActor struct Issue71 {
    struct Field: Sendable, CustomTestStringConvertible {
        let type: String
        let source: String
        let filled: JSONValue
        var testDescription: String { type }
    }

    nonisolated static let fields: [Field] = [
        Field(type: "TextArea", source: #"TextArea("x", "Write", 3, {required: true})"#, filled: .string("hello\nworld")),
        Field(type: "RadioGroup", source: #"RadioGroup("x", [RadioItem("Alpha", "", "a"), RadioItem("Beta", "", "b")], null, {required: true})"#, filled: .string("a")),
        Field(type: "CheckBoxGroup", source: #"CheckBoxGroup("x", [CheckBoxItem("Alpha", "", "a")], {required: true})"#, filled: .object([("a", .bool(true))])),
        Field(type: "Slider", source: #"Slider("x", "discrete", 0, 10, 1, null, "Amount", {required: true})"#, filled: .array([.number(7)])),
        Field(type: "DatePicker", source: #"DatePicker("x", "single", {required: true})"#, filled: .string("2026-11-17T00:00:00.000Z")),
        Field(type: "Chips", source: #"Chips("x", "multiple", [ChipItem("a", "Alpha")], {required: true})"#, filled: .array([.string("a")])),
        Field(type: "OptionCards", source: #"OptionCards("x", "single", [OptionCard("a", "Alpha")], {required: true})"#, filled: .string("a")),
    ]

    func program(_ field: Field, bound: Bool = false) async throws -> LiveProgram {
        let core = Core()
        let (library, spec) = try await reactLibrary(core)
        var source = field.source
        if bound { source = String(source.dropLast()) + ", $v)" }
        let live = try LiveProgram(library: library, promptSpec: spec)
        NativeTestWatchdog.waitingFor("Core to parse the \(field.type) form")
        try live.show(try await core.parseOneShot("$v = null\nroot = Form(\"f\", Buttons([Button(\"Send\")]), [\(source)])", library: library))
        return live
    }

    func observations() throws -> [JSONValue] {
        var directory = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { directory.deleteLastPathComponent() }
        let json = try JSONValue.parse(String(contentsOf: directory.appendingPathComponent("tools/viewer/Checks/field-observations.json"), encoding: .utf8))
        return try #require(json.array)
    }

    @Test(arguments: fields)
    func aNewControlsDrawingChangesWithItsCurrentValue(_ field: Field) async throws {
        let live = try await program(field)
        let window = await Window(live)
        defer { window.close() }
        func pixels() throws -> Data {
            let bitmap = try #require(window.host.bitmapImageRepForCachingDisplay(in: window.host.bounds))
            window.host.cacheDisplay(in: window.host.bounds, to: bitmap)
            return try #require(bitmap.representation(using: .png, properties: [:]))
        }
        let before = try pixels()
        try live.enter(field.filled, into: try #require(element(live, at: "/fields/0")), form: "f")
        await window.settle()
        #expect(try pixels() != before)
    }

    @Test(arguments: fields)
    func anAppCanReplaceEveryNewControl(_ field: Field) async throws {
        let live = try await program(field)
        var replaced = false
        let drawing = ElementDrawing(component: { element, _ in
            guard element.typeName == field.type else { return nil }
            replaced = true
            return AnyView(Text("App control"))
        })
        let renderer = ImageRenderer(content: ElementView(live: live).environment(\.elementDrawing, drawing))
        renderer.proposedSize = ProposedViewSize(width: 400, height: nil)
        #expect(renderer.nsImage != nil)
        #expect(replaced)
    }

    /// Entry goes through the same public method used by native controls;
    /// mounting and physical interactions are checked separately below.
    @Test(NativeTestWatchdog.native, arguments: fields)
    func everyNewControlWritesItsFormShapeAndUnblocksSend(_ field: Field) async throws {
        let observation = try #require(try observations().first { $0["type"] == .string(field.type) })
        #expect(observation["value"] == field.filled)
        let live = try await program(field)
        var events: [HostEvent] = []
        live.onHostEvent = { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        let input = try #require(element(live, at: "/fields/0"))
        #expect(NativeComponents.view(for: input, path: "/fields/0") != nil)
        let before = live.version
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        #expect(live.version == before)
        try live.enter(field.filled, into: input, form: "f")
        #expect(live.field(form: "f", name: "x") == field.filled)
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
        let formState = try #require(events.first?.formState)
        let stored = try #require(formState["f"]?["x"])
        #expect(stored["value"] == field.filled)
        let componentType = observation["componentType"]
        #expect(stored["componentType"] == componentType)
        live.isStreaming = true
        let version = live.version
        #expect(throws: OpenUIError.self) { try live.enter(.string("refused"), into: input, form: "f") }
        #expect(live.version == version)
        #expect(live.field(form: "f", name: "x") == field.filled)
    }

    @Test(arguments: fields.filter { !["Chips", "OptionCards"].contains($0.type) })
    func newStateFieldsWriteTheirBindingAndRefuseStaleElements(_ field: Field) async throws {
        let live = try await program(field, bound: true)
        let input = try #require(element(live, at: "/fields/0"))
        try live.enter(field.filled, into: input, form: "f")
        #expect(live.value(of: "$v") == field.filled)
        #expect(live.field(form: "f", name: "x") == nil)
        let core = Core()
        let (library, _) = try await reactLibrary(core)
        try live.show(try await core.parseOneShot("root = TextContent(\"gone\")", library: library))
        let before = live.state
        let version = live.version
        #expect(throws: OpenUIError.self) { try live.enter(.string("refused"), into: input, form: "f") }
        #expect(live.state == before)
        #expect(live.version == version)
    }

    @Test(arguments: ["RadioGroup", "CheckBoxGroup", "Chips", "OptionCards"])
    func clickingAChoiceWritesItsNativeControl(_ type: String) async throws {
        let field = try #require(Self.fields.first { $0.type == type })
        let live = try await program(field)
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/fields/0", at: { CGPoint(x: $0.minX + 8, y: $0.minY + 9) })
        #expect(live.field(form: "f", name: "x") == field.filled)
        #expect(live.failure == nil)
    }

    @Test func typingInTheTextEditorWritesItsBinding() async throws {
        let live = try await program(try #require(Self.fields.first), bound: true)
        let window = await Window(live)
        defer { window.close() }
        func editors(_ view: NSView) -> [NSTextView] {
            (view as? NSTextView).map { [$0] } ?? view.subviews.flatMap(editors)
        }
        let editor = try #require(editors(window.host).first)
        #expect(window.window.makeFirstResponder(editor))
        editor.insertText("hello\nworld", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        await window.settle()
        #expect(live.value(of: "$v") == .string("hello\nworld"))
    }

    @Test func movingTheSystemSliderWritesANumberArray() async throws {
        let sample = try #require(Self.fields.first { $0.type == "Slider" })
        // Both variants snap to a positive program step; continuous has no SwiftUI ticks.
        for source in [sample.source, sample.source.replacingOccurrences(of: "discrete", with: "continuous").replacingOccurrences(of: "0, 10, 1,", with: "0, 10, 1,")] {
            let field = Field(type: "Slider", source: source, filled: sample.filled)
            let live = try await program(field, bound: true)
            let window = await Window(live)
            defer { window.close() }
            try await window.click("/fields/0", at: { CGPoint(x: $0.minX + $0.width * 0.75, y: $0.maxY - 8) })
            let numbers = try #require(live.value(of: "$v")?.array)
            #expect(numbers.count == 1)
            let number = try #require(numbers.first?.number)
            #expect(number > 0 && number <= 10 && number == number.rounded())
        }
    }

    @Test func editingTheSystemDatePickerWritesAnISODate() async throws {
        let live = try await program(try #require(Self.fields.first { $0.type == "DatePicker" }), bound: true)
        try live.set("$v", to: .string("2026-11-16T00:00:00.000Z"))
        let zone = try #require(TimeZone(secondsFromGMT: 0))
        let window = await Window(live, component: { element, path in
            guard element.typeName == "DatePicker" else { return nil }
            return NativeComponents.view(for: element, path: path).map { AnyView($0.environment(\.timeZone, zone)) }
        })
        defer { window.close() }
        func pickers(_ view: NSView) -> [NSDatePicker] {
            (view as? NSDatePicker).map { [$0] } ?? view.subviews.flatMap(pickers)
        }
        let picker = try #require(pickers(window.host).first)
        let date = try #require(ISO8601DateFormatter().date(from: "2026-11-17T00:00:00Z"))
        picker.dateValue = date
        #expect(picker.sendAction(picker.action, to: picker.target))
        await window.settle()
        #expect(live.value(of: "$v") == .string("2026-11-17T00:00:00.000Z"))
    }

    @Test(arguments: ["RadioGroup", "Slider", "legacySlider"])
    func visualDefaultsMatchTheirExplicitStoredValues(_ type: String) async throws {
        let live: LiveProgram
        let form: String?
        let initial: JSONValue
        let changed: JSONValue
        if type == "legacySlider" {
            var directory = URL(fileURLWithPath: #filePath)
            for _ in 0..<5 { directory.deleteLastPathComponent() }
            let core = Core()
            let library = try await core.library(schema: String(contentsOf: directory.appendingPathComponent("conformance/imported/schemas/bench.json"), encoding: .utf8))
            live = try LiveProgram(library: library)
            try live.show(try await core.parseOneShot(#"root = Slider("x", "discrete", 0, 10, 1, 4)"#, library: library))
            form = nil
            initial = .array([.number(4)])
            changed = .array([.number(7)])
        } else {
            let source = type == "RadioGroup"
                ? #"RadioGroup("x", [RadioItem("Alpha", "", "a"), RadioItem("Beta", "", "b")], "a")"#
                : #"Slider("x", "discrete", 0, 10, 1, [4])"#
            initial = type == "RadioGroup" ? .string("a") : .array([.number(4)])
            changed = type == "RadioGroup" ? .string("b") : .array([.number(7)])
            live = try await program(Field(type: type, source: source, filled: initial))
            form = "f"
        }
        let window = await Window(live)
        defer { window.close() }
        func pixels() throws -> Data {
            let bitmap = try #require(window.host.bitmapImageRepForCachingDisplay(in: window.host.bounds))
            window.host.cacheDisplay(in: window.host.bounds, to: bitmap)
            return try #require(bitmap.representation(using: .png, properties: [:]))
        }
        #expect(live.field(form: form, name: "x") == nil)
        let before = try pixels()
        try live.setField(form: form, name: "x", to: initial)
        await window.settle()
        #expect(try pixels() == before)
        try live.setField(form: form, name: "x", to: changed)
        await window.settle()
        #expect(try pixels() != before)
    }

    @Test func invalidSliderBoundsRefuseBeforeConstructingASystemControl() throws {
        for (lower, upper, step) in [("5", "0", "1"), ("0", "0", "1"), ("0", "10", "0"), ("0", "10", "-1"), ("1e309", "2e309", "1"), ("0", "10", "1e309"), ("-1e308", "1e308", "1")] {
            let json = """
            {"root":{"type":"element","typeName":"Slider","props":{"name":"x","variant":"discrete","min":\(lower),"max":\(upper),"step":\(step)},"partial":false,"hasDynamicProps":false},"meta":{}}
            """
            let element = try #require(ParseResult(json: json, depthLimited: false, sizeLimited: false).root)
            #expect(NativeComponents.view(for: element, path: "/") == nil)
            #expect(draw(element, with: ElementDrawing()))
        }
    }

    @Test func selectionDefaultsHydrateMetadataAndSingleChoicesCanBeCleared() async throws {
        let core = Core()
        let (library, spec) = try await reactLibrary(core)
        let live = try LiveProgram(library: library, promptSpec: spec)
        try live.show(try await core.parseOneShot(#"root = Chips("x", "single", [ChipItem("a", "Alpha")], null, ["a", "b"])"#, library: library))
        let window = await Window(live)
        defer { window.close() }
        #expect(live.field(form: nil, name: "x") == .string("a"))
        #expect(live.state["x"]?["componentType"] == .string("Chips"))
        try await window.click("/", at: { CGPoint(x: $0.minX + 8, y: $0.minY + 9) })
        #expect(live.field(form: nil, name: "x") == nil)
        #expect(live.failure == nil)
    }

    @Test func aRequiredRadioDefaultCanBeChosenOnScreen() async throws {
        let core = Core()
        let (library, spec) = try await reactLibrary(core)
        var events: [HostEvent] = []
        let live = try LiveProgram(library: library, promptSpec: spec) { events.append($0) }
        try live.show(try await core.parseOneShot(#"root = Form("f", Buttons([Button("Send")]), [RadioGroup("x", [RadioItem("Alpha", "", "a")], "a", {required: true})])"#, library: library))
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        try await window.click("/fields/0", at: { CGPoint(x: $0.minX + 8, y: $0.minY + 9) })
        #expect(live.field(form: "f", name: "x") == .string("a"))
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
    }

    @Test func disabledSelectionItemsRefuseClicksWithoutChangingState() async throws {
        let core = Core()
        let (library, spec) = try await reactLibrary(core)
        let live = try LiveProgram(library: library, promptSpec: spec)
        try live.show(try await core.parseOneShot(#"root = Chips("x", "multiple", [ChipItem("a", "Alpha", null, true)])"#, library: library))
        let window = await Window(live)
        defer { window.close() }
        let state = live.state
        let version = live.version
        try await window.click("/", at: { CGPoint(x: $0.minX + 8, y: $0.minY + 9) })
        #expect(live.state == state && live.version == version)
    }

    @Test func duplicateCheckboxNamesShareTheLastDefaultAndOneStoredBoolean() async throws {
        let core = Core()
        let (library, spec) = try await reactLibrary(core)
        let live = try LiveProgram(library: library, promptSpec: spec)
        try live.show(try await core.parseOneShot(#"root = CheckBoxGroup("x", [CheckBoxItem("First", "", "a", false), CheckBoxItem("Second", "", "a", true)])"#, library: library))
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/", at: { CGPoint(x: $0.minX + 8, y: $0.minY + 9) })
        let observation = try #require(try observations().first { $0["scenario"] == .string("duplicateNames") })
        #expect(live.field(form: nil, name: "x") == observation["value"])
    }

    @Test func dateRangesKeepFromAndToAsISOStrings() async throws {
        let core = Core()
        let (library, spec) = try await reactLibrary(core)
        let live = try LiveProgram(library: library, promptSpec: spec)
        try live.show(try await core.parseOneShot(#"""
        $range = {from: "2026-11-16T00:00:00.000Z", to: "2026-11-19T00:00:00.000Z"}
        root = DatePicker("x", "range", null, $range)
        """#, library: library))
        let zone = try #require(TimeZone(secondsFromGMT: 0))
        let window = await Window(live, component: { element, path in
            guard element.typeName == "DatePicker" else { return nil }
            return NativeComponents.view(for: element, path: path).map { AnyView($0.environment(\.timeZone, zone)) }
        })
        defer { window.close() }
        func pickers(_ view: NSView) -> [NSDatePicker] {
            (view as? NSDatePicker).map { [$0] } ?? view.subviews.flatMap(pickers)
        }
        let controls = pickers(window.host)
        #expect(controls.count == 2)
        for (index, text) in [(0, "2026-11-17T00:00:00Z"), (1, "2026-11-20T00:00:00Z"), (0, "2026-11-18T00:00:00Z"), (0, "2026-11-17T00:00:00Z")] {
            let control = try #require(controls.indices.contains(index) ? controls[index] : nil)
            control.dateValue = try #require(ISO8601DateFormatter().date(from: text))
            #expect(control.sendAction(control.action, to: control.target))
            await window.settle()
        }
        let observation = try #require(try observations().first { $0["mode"] == .string("range") })
        #expect(live.value(of: "$range") == observation["value"])
    }

    @Test func upstreamValidationObservationsMatchNativeSendChecks() async throws {
        var directory = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { directory.deleteLastPathComponent() }
        let fixture = try JSONValue.parse(String(contentsOf: directory.appendingPathComponent("tools/viewer/Checks/validation-observations.json"), encoding: .utf8))
        let cases = try #require(fixture.array)
        #expect(cases.count >= 40)
        for c in cases {
            let rules = try #require(c["rules"])
            let value = try #require(c["value"])
            let blocks = try #require(c["blocks"]?.bool as Bool?)
            var events: [HostEvent] = []
            let live = try await live("root = Form(\"f\", [Input(\"x\", \(value.compact), \(rules.compact))], Buttons([Button(\"Send\")]))") { events.append($0) }
            let window = await Window(live)
            try await window.click("/buttons/buttons/0")
            #expect(events.count == (blocks ? 0 : 1), "\(c.compact)")
            window.close()
        }
    }

    @Test func documentedValidationRemaindersStayExplicit() async throws {
        var directory = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { directory.deleteLastPathComponent() }
        let fixture = try JSONValue.parse(String(contentsOf: directory.appendingPathComponent("tools/viewer/Checks/validation-remainders.json"), encoding: .utf8))
        let cases = try #require(fixture.array)
        #expect(cases.count == 4)
        for c in cases {
            let rules = try #require(c["rules"])
            let value = try #require(c["value"])
            let nativeBlocks = try #require(c["nativeBlocks"]?.bool as Bool?)
            #expect(c["blocks"]?.bool != nativeBlocks)
            var events: [HostEvent] = []
            let live = try await live("root = Form(\"f\", [Input(\"x\", \(value.compact), \(rules.compact))], Buttons([Button(\"Send\")]))") { events.append($0) }
            let window = await Window(live)
            try await window.click("/buttons/buttons/0")
            #expect(events.count == (nativeBlocks ? 0 : 1), "\(c.compact)")
            window.close()
        }
    }

    @Test func refusedPersistedValuesLeaveThePreviousProgramClickableAndStateUnchanged() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library)
        let original = try await core.parseOneShot("$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))", library: library)
        try live.show(original)
        let button = try #require(live.root)
        try live.run("action", of: button, message: "Go")
        let replacement = try await core.parseOneShot("$n = 99\n$new = 3\nroot = Button(\"Wrong\", @Set($n, 100))", library: library)
        for persisted in [JSONValue.array([]), .string("bad"), .object([("big", .string(String(repeating: "x", count: 9 << 20)))])] {
            let state = live.state
            let version = live.version
            let pristine = live.pristineNames
            #expect(throws: (any Error).self) { try live.show(replacement, persisted: persisted) }
            #expect(live.state == state)
            #expect(live.version == version)
            #expect(live.pristineNames == pristine)
            #expect(live.acceptsInteractions)
            try live.run("action", of: button, message: "Go")
        }
        #expect(live.value(of: "$n") == .number(4))
        #expect(live.value(of: "$new") == nil)
    }

    @Test func discardingPreviewsRestoresCompleteFieldsPristineStateAndPendingMutations() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        var events: [HostEvent] = []
        let live = try LiveProgram(library: library) { events.append($0) }
        let source = """
        $n = 0
        $pristine = 7
        m = Mutation("t")
        root = Form("f", [Input("x")], Buttons([Button("Go", @Set($n, $n + 1)), Button("Wait", Action([@Run(m), @Set($n, $n + 10)]))]))
        """
        try live.show(try await core.parseOneShot(source, library: library))
        let button = try #require(element(live, at: "/buttons/buttons/0"))
        let input = try #require(element(live, at: "/fields/0"))
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        try live.enter(.string("Ann"), into: input, form: "f")
        try live.run("action", of: try #require(element(live, at: "/buttons/buttons/1")), message: "Wait", form: "f")
        let pending = try #require(events.last)
        #expect(pending.awaitsResume)
        let state = live.state
        let version = live.version
        let pristine = live.pristineNames
        let stream = try await core.session(library: library)
        let preview = try await stream.push([source.replacingOccurrences(of: "$pristine = 7", with: "$pristine = 99\n$new = 3")])
        try live.show(preview)
        let partialButton = try #require(element(live, at: "/buttons/buttons/0"))
        #expect(live.value(of: "$pristine") == .number(99))
        #expect(throws: OpenUIError.self) { try live.run("action", of: partialButton, message: "Go", form: "f") }
        try await window.click("/buttons/buttons/0")
        #expect(live.value(of: "$n") == .number(1))
        live.discardPreview()
        #expect(live.state == state && live.version == version)
        #expect(live.pristineNames == pristine)
        #expect(live.field(form: "f", name: "x") == .string("Ann"))
        #expect(live.value(of: "$new") == nil)
        #expect(throws: OpenUIError.self) { try live.run("action", of: partialButton, message: "Go", form: "f") }
        #expect(live.state == state && live.version == version)
        try live.run("action", of: button, message: "Go", form: "f")
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(live.value(of: "$n") == .number(3))
        try live.resume(pending, succeeded: true)
        #expect(live.value(of: "$n") == .number(13))
    }

    @Test func appEditsDuringPreviewSurviveCompletionEvenWhenEqualToTheOldDefault() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library)
        try live.show(try await core.parseOneShot("$n = 1\nroot = TextContent($n)", library: library))
        let stream = try await core.session(library: library)
        try live.show(try await stream.push(["$n = 2\nroot = TextContent($n)"]))
        #expect(live.value(of: "$n") == .number(2))
        try live.set("$n", to: .number(1))
        #expect(live.value(of: "$n") == .number(1))
        try live.show(try await stream.push(["\nextra = 3"]))
        #expect(live.value(of: "$n") == .number(1))
        try live.show(try await stream.finish())
        #expect(live.value(of: "$n") == .number(1))
        #expect(!live.pristineNames.contains("$n"))
    }

    @Test func tabsCannotChangeTheCompleteFormsValidationWhileAPreviewIsDrawn() async throws {
        let core = Core()
        let library = try await core.library(schema: Issue63.extendedSchema)
        var events: [HostEvent] = []
        let live = try LiveProgram(library: library) { events.append($0) }
        let source = #"root = Form("f", [Tabs([TabItem("First", [Input("a", "ok", {required: true})]), TabItem("Second", [Input("b", null, {required: true})])])], Buttons([Button("Send")]))"#
        try live.show(try await core.parseOneShot(source, library: library))
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
        let stream = try await core.session(library: library)
        try live.show(try await stream.push([source]))
        await window.settle()
        #expect(!live.acceptsInteractions)
        try await window.click("/fields/0", at: { CGPoint(x: $0.minX + 95, y: $0.minY + 12) })
        live.discardPreview()
        await window.settle()
        #expect(live.acceptsInteractions)
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 2)
    }

    @Test func discardingAPreviewPreservesStateHeldCustomActionAuthority() async throws {
        let core = Core()
        let schema = liveSchema.replacingOccurrences(of: "\"$defs\":", with: #""actions":{"Ping":{"params":{"type":"object","properties":{}}}},"$defs":"#)
        let library = try await core.library(schema: schema)
        var events: [HostEvent] = []
        let live = try LiveProgram(library: library) { events.append($0) }
        let source = """
        $plan = null
        root = Stack([Button("Store", @Set($plan, @Ping())), Button("Send", $plan)])
        """
        try live.show(try await core.parseOneShot(source, library: library))
        try live.run("action", of: try #require(element(live, at: "/children/0")), message: "Store")
        let send = try #require(element(live, at: "/children/1"))
        try live.run("action", of: send, message: "Send")
        #expect(events.last?.type == "Ping")
        let stream = try await core.session(library: library)
        try live.show(try await stream.push([source + "\nextra = 3"]))
        live.discardPreview()
        try live.run("action", of: send, message: "Send")
        #expect(events.count == 2)
        #expect(events.last?.type == "Ping")
        #expect(live.failure == nil)
    }

    @Test func aPreviewShowThatThrowsRestoresTheCompleteProgram() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library)
        try live.show(try await core.parseOneShot("$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))", library: library))
        let original = try #require(live.root)
        try live.run("action", of: original, message: "Go")
        let state = live.state
        let version = live.version
        let stream = try await core.session(library: library)
        let preview = try await stream.push(["$new = 3\nroot = Button(\"Wrong\", @Set($n, 99))"])
        #expect(throws: (any Error).self) { try live.show(preview, persisted: .string("invalid")) }
        #expect(live.state == state && live.version == version)
        #expect(!live.isStreaming)
        try live.run("action", of: original, message: "Go")
        #expect(live.value(of: "$n") == .number(2))
    }

    @Test func aStreamPushNeverBecomesClickableWhenTheAppClearsItsFlag() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library, bindings: [("Input", "value")])
        let stream = try await core.session(library: library)
        let partial = try await stream.push(["$n = 0\nroot = Stack([Button(\"Go\", @Set($n, 9)), Input(\"x\", $n)])"])
        #expect(partial.isStreaming)
        try live.show(partial)
        live.isStreaming = false
        let button = try #require(element(live, at: "/children/0"))
        let input = try #require(element(live, at: "/children/1"))
        let state = live.state
        let version = live.version
        #expect(live.isStreaming)
        #expect(throws: OpenUIError.self) { try live.run("action", of: button, message: "Go") }
        #expect(throws: OpenUIError.self) { try live.enter(.number(9), into: input) }
        guard case let .binding(marker)? = input.props.first(where: { $0.name == "value" })?.value else { Issue.record("missing marker"); return }
        #expect(throws: OpenUIError.self) { try live.write(marker, .number(9)) }
        #expect(live.state == state)
        #expect(live.version == version)
        let final = try await stream.finish()
        #expect(!final.isStreaming)
        try live.show(final)
        try live.run("action", of: try #require(element(live, at: "/children/0")), message: "Go")
        #expect(live.value(of: "$n") == .number(9))
    }
}
}
