// A click runs exactly the action that was drawn, or nothing; writes from
// inputs and markers likewise. Props decode as what they are.
import AppKit
import OpenUINative
import SwiftUI
import Testing

/// The element at `path` in the live program's evaluated tree.
@MainActor func element(_ live: LiveProgram, at path: String) -> Element? {
    guard var value = live.root.map(PropValue.element) else { return nil }
    for part in path.split(separator: "/").map(String.init) {
        switch value {
        case let .element(e): guard let next = e.props.first(where: { $0.name == part })?.value else { return nil }; value = next
        case let .list(items): guard let i = Int(part), items.indices.contains(i) else { return nil }; value = items[i]
        case let .record(fields): guard let next = fields.first(where: { $0.name == part })?.value else { return nil }; value = next
        default: return nil
        }
    }
    if case let .element(e) = value { return e }
    return nil
}

@Suite @MainActor struct AClickRunsWhatWasDrawn {
    /// An app's component draws its child with `ElementView(element:)` and
    /// no path: the child's own path is used.
    @Test func aButtonAnAppsComponentDrawsRunsItsOwnAction() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        $deleted = false
        root = Stack([TextContent("x"), Panel(Button("Delete", @Set($deleted, true)))])
        """) { events.append($0) }
        let window = Window(live, component: { element, _ in
            guard element.typeName == "Panel", case let .element(child)? = element.props.first?.value else { return nil }
            return AnyView(ElementView(element: child))
        }, key: { element, path in element.typeName == "Button" ? "button" : path })
        defer { window.close() }
        try window.click("button")
        #expect(live.value(of: "$deleted") == .bool(true))
        #expect(events.isEmpty)
        #expect(live.failure == nil)
    }

    /// Two clicks arrive before the redraw, and the first changes what's at
    /// the place clicked: the second runs nothing and says why.
    @Test func aSecondClickBeforeTheRedrawRunsNothing() async throws {
        let live = try await live("""
        $first = true
        $two = 0
        root = Stack($first ? [Button("One", @Set($first, false)), Button("Two", @Set($two, 1))] : [Button("Two", @Set($two, 1)), Button("Three", @Set($two, 3))])
        """)
        let window = Window(live)
        defer { window.close() }
        for event in try window.clickEvents("/children/0", clicks: 2) { window.window.sendEvent(event) }
        window.settle()
        #expect(live.value(of: "$first") == .bool(false))
        #expect(live.value(of: "$two") == .number(0))
        #expect(live.failure?.contains("changed since this was drawn") == true)
        try window.click("/children/0")
        #expect(live.value(of: "$two") == .number(1))
        #expect(live.failure == nil)
    }

    /// Two clicks arrive before the redraw and the first leaves the button
    /// exactly as it was drawn: the second runs too, as it would on a page.
    @Test func aSecondClickOnAnUnchangedButtonBeforeTheRedrawRunsAgain() async throws {
        let live = try await live("""
        $n = 0
        root = Stack([TextContent("n: " + $n), Button("Add", @Set($n, $n + 1))])
        """)
        let window = Window(live)
        defer { window.close() }
        for event in try window.clickEvents("/children/1", clicks: 2) { window.window.sendEvent(event) }
        window.settle()
        #expect(live.value(of: "$n") == .number(2))
        #expect(live.failure == nil)
        #expect(window.texts["/children/0"] == "n: 2")
    }

    /// Two keystrokes arrive before the redraw: both are written.
    @Test func twoKeystrokesBeforeTheRedrawAreBothWritten() async throws {
        let live = try await live("""
        $q = ""
        root = Stack([TextContent("q: " + $q), Input("q", $q)])
        """)
        let window = Window(live)
        defer { window.close() }
        try window.typeAtOnce(["a", "b"])
        #expect(live.value(of: "$q") == .string("ab"))
        #expect(live.failure == nil)
        #expect(window.texts["/children/0"] == "q: ab")
    }

    /// A button drawn in a form that has since been renamed runs nothing,
    /// though the button itself is unchanged: its event would carry another
    /// form's fields.
    @Test func anUnchangedButtonInARenamedFormRunsNothing() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        $f = "a"
        root = Form($f, [], Buttons([Button("Send", @ToAssistant("sent"))]))
        """) { events.append($0) }
        let drawn = try #require(element(live, at: "/buttons/buttons/0"))
        try live.set("$f", to: .string("b"))
        #expect(throws: LiveError.self) { try live.run("action", of: drawn, message: "Send", form: "a") }
        #expect(events.isEmpty)
        try live.run("action", of: try #require(element(live, at: "/buttons/buttons/0")), message: "Send", form: "b")
        #expect(events.map(\.formName) == ["b"])
    }

    /// The field on screen was drawn for one name, and the tree now has
    /// another there: what's typed in it is refused, not written to either.
    @Test func anUnboundFieldWhoseInputChangedRefusesAStaleWrite() async throws {
        let live = try await live("""
        $name = "first"
        root = Form("f", [FormControl("Name", Input($name))])
        """)
        let window = Window(live)
        defer { window.close() }
        try live.set("$name", to: .string("second"))
        let field = try #require(window.fields.first)
        #expect(window.window.makeFirstResponder(field))
        try #require(field.currentEditor()).insertText("x")
        #expect(live.field(form: "f", name: "first") == nil)
        #expect(live.field(form: "f", name: "second") == nil)
        #expect(live.failure?.contains("changed since this was drawn") == true)
    }

    @Test func eachRowsButtonRunsItsOwnRow() async throws {
        let live = try await live("""
        $items = ["a", "b", "c"]
        $picked = ""
        root = Stack(@Each($items, "x", Button(x, @Set($picked, x))))
        """)
        let window = Window(live)
        defer { window.close() }
        try window.click("/children/2")
        #expect(live.value(of: "$picked") == .string("c"))
        try window.click("/children/0")
        #expect(live.value(of: "$picked") == .string("a"))
    }

    @Test func aNestedButtonRunsItsAction() async throws {
        let live = try await live("""
        $a = 0
        root = Stack([TextContent("t"), Stack([TextContent("u"), Button("in", @Set($a, 1))])])
        """)
        let window = Window(live)
        defer { window.close() }
        try window.click("/children/1/children/1")
        #expect(live.value(of: "$a") == .number(1))
    }

    @Test func anInputAnAppsComponentDrawsWritesItsOwnBinding() async throws {
        let live = try await live("""
        $q = ""
        root = Stack([TextContent("q: " + $q), Panel(Input("q", $q))])
        """)
        let window = Window(live, component: { element, _ in
            guard element.typeName == "Panel", case let .element(child)? = element.props.first?.value else { return nil }
            return AnyView(ElementView(element: child))
        })
        defer { window.close() }
        try window.type("hi")
        #expect(live.value(of: "$q") == .string("hi"))
        #expect(window.texts["/children/0"] == "q: hi")
    }

    /// An element from an earlier evaluation runs only while the element at
    /// its place now is exactly the same.
    @Test func anElementFromAnEarlierEvaluationRunsOnlyWhileItIsStillWhatIsThere() async throws {
        let live = try await live("""
        $n = 0
        root = Stack([Button("Add to " + $n, @Set($n, $n + 1)), Button("Reset", @Set($n, 0))])
        """)
        let add = try #require(element(live, at: "/children/0"))
        let reset = try #require(element(live, at: "/children/1"))
        #expect(live.isCurrent(add))
        try live.set("$n", to: .number(5))
        #expect(!live.isCurrent(add))
        #expect(throws: LiveError.self) { try live.run("action", of: add, message: "") }
        #expect(live.value(of: "$n") == .number(5))
        try live.run("action", of: reset, message: "")
        #expect(live.value(of: "$n") == .number(0))
        try live.run("action", of: try #require(element(live, at: "/children/0")), message: "")
        #expect(live.value(of: "$n") == .number(1))
    }

    @Test func anElementFromAnotherProgramRunsNothing() async throws {
        let program = "$n = 0\nroot = Stack([Button(\"Add\", @Set($n, $n + 1))])"
        let one = try await live(program)
        let two = try await live(program)
        let theirs = try #require(element(two, at: "/children/0"))
        #expect(throws: LiveError.self) { try one.run("action", of: theirs, message: "") }
        #expect(one.value(of: "$n") == .number(0))
        #expect(two.value(of: "$n") == .number(0))
    }

    @Test func aMarkerFromAnEarlierEvaluationWritesNothingOnceItIsGone() async throws {
        let live = try await live("""
        $shown = true
        $hp = 5
        root = Stack($shown ? [Gauge(1, $hp, null)] : [TextContent("gone")])
        """, bindings: [("Gauge", "value")])
        guard case let .binding(marker)? = element(live, at: "/children/0")?.props.first(where: { $0.name == "value" })?.value else {
            Issue.record("no marker"); return
        }
        try live.set("$other", to: .number(1))
        try live.write(marker, .number(6))
        #expect(live.value(of: "$hp") == .number(6))
        try live.set("$shown", to: .bool(false))
        #expect(throws: LiveError.self) { try live.write(marker, .number(9)) }
        #expect(live.value(of: "$hp") == .number(6))
    }
}

@Suite @MainActor struct DrawingAtPaths {
    /// An app's component draws its child with a path of its own choosing;
    /// the child, from an evaluated tree, is drawn at its own path anyway.
    @Test func anElementOfAnEvaluatedTreeIsDrawnAtItsOwnPath() async throws {
        let live = try await live(#"root = Stack([TextContent("x"), Panel(TextContent("inside"))])"#)
        var paths: [String: String] = [:]
        let drawing = ElementDrawing(component: { element, _ in
            guard element.typeName == "Panel", case let .element(child)? = element.props.first?.value else { return nil }
            return AnyView(ElementView(element: child, path: "/somewhere/else"))
        }, chrome: { view, element, path in
            if case let .scalar(.string(text))? = element.props.first?.value { paths[text] = path }
            return view
        })
        _ = ImageRenderer(content: ElementView(live: live).environment(\.elementDrawing, drawing)).nsImage
        #expect(paths == ["x": "/children/0", "inside": "/children/1/child"])
    }
}

@Suite @MainActor struct DecodingProps {
    /// Only the runtime's own marker, exactly `{__reactive: "assign", target,
    /// expr}`, is a marker; data that comes close stays data.
    @Test func onlyTheExactMarkerShapeIsAMarker() async throws {
        let live = try await live("""
        $hp = 1
        $near = [{__reactive: "assign", target: "$hp"}, {__reactive: "assign", target: "$hp", expr: 1, more: true}, {target: "$hp", __reactive: "assign", expr: 1}, {__reactive: "set", target: "$hp", expr: 1}, {__reactive: "assign", target: 1, expr: 1}]
        root = Stack([Gauge(1, $hp, null), Panel($near)])
        """, bindings: [("Gauge", "value")])
        guard case let .binding(marker)? = element(live, at: "/children/0")?.props.first(where: { $0.name == "value" })?.value,
              case let .object(entries) = marker.json else {
            Issue.record("the runtime's marker isn't a marker"); return
        }
        #expect(entries.map(\.key) == ["__reactive", "target", "expr"])
        #expect(marker.target == "$hp")
        guard case let .list(near)? = element(live, at: "/children/1")?.props.first?.value else {
            Issue.record("no data"); return
        }
        #expect(near.count == 5)
        for value in near {
            guard case .record = value else { Issue.record("\(value.summary) isn't data"); continue }
        }
    }

    /// Data with a `steps` key is data: all its keys reach a component.
    @Test func dataWithAStepsKeyIsDataNotAPlan() async throws {
        let live = try await live("""
        $obj = {steps: [1, 2], name: "x"}
        root = Stack([Gauge(1, $obj, null)])
        """, bindings: [])
        guard case let .record(fields)? = element(live, at: "/children/0")?.props.first(where: { $0.name == "value" })?.value else {
            Issue.record("not a record"); return
        }
        #expect(fields.map(\.name) == ["steps", "name"])
        #expect(fields.last?.value.summary == #""x""#)
    }

    /// Something shaped like a plan but not the session's own plan there, or
    /// data with steps, runs nothing when clicked.
    @Test func aForgedPlanRunsNothing() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        $x = 0
        root = Stack([Button("a", {steps: [1, {type: "set"}], type: "open_url", url: "u"}), Button("b", {steps: [{type: "custom_action", name: "Ping", params: {}}]})])
        """) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        let before = live.version
        try window.click("/children/0")
        #expect(live.failure != nil)
        try window.click("/children/1")
        #expect(events.isEmpty)
        #expect(live.version == before)
    }
}

@Suite @MainActor struct MutationsAndRefusals {
    static let program = """
    $x = 0
    m = Mutation("t")
    root = Stack([TextContent("x: " + $x), Button("Go", Action([@Run(m), @Set($x, $x + 1)]))])
    """

    @Test func resumeCarriesOnOrStopsAndOnlyOnce() async throws {
        var events: [HostEvent] = []
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library) { events.append($0) }
        try live.show(try await core.parseOneShot(Self.program, library: library))
        func go() throws -> HostEvent {
            events = []
            try live.run("action", of: try #require(element(live, at: "/children/1")), message: "Go")
            return try #require(events.last)
        }
        let first = try go()
        #expect(first.type == "run")
        #expect(first.awaitsResume)
        #expect(live.value(of: "$x") == .number(0))
        try live.resume(first, succeeded: true)
        #expect(live.value(of: "$x") == .number(1))
        #expect(texts(drawing: live)["/children/0"] == "x: 1")
        #expect(throws: (any Error).self) { try live.resume(first, succeeded: true) }
        #expect(live.value(of: "$x") == .number(1))

        try live.resume(try go(), succeeded: false)
        #expect(live.value(of: "$x") == .number(1))

        let pending = try go()
        try live.show(try await core.parseOneShot(Self.program + "\nextra = TextContent(\"e\")", library: library))
        try live.resume(pending, succeeded: true)
        #expect(live.value(of: "$x") == .number(2))
        #expect(texts(drawing: live)["/children/0"] == "x: 2")

        var others: [HostEvent] = []
        let other = try LiveProgram(library: library) { others.append($0) }
        try other.show(try await core.parseOneShot(Self.program, library: library))
        try other.run("action", of: try #require(element(other, at: "/children/1")), message: "Go")
        let foreign = try #require(others.last)
        #expect(throws: (any Error).self) { try live.resume(foreign, succeeded: true) }
        #expect(live.value(of: "$x") == .number(2))
        #expect(other.value(of: "$x") == .number(0))
    }

    /// A step the store refuses stops the run: earlier steps stay, the rest
    /// don't run, `failure` says why, and the drawing matches the store.
    @Test func aRefusedStepSurfacesAndChangesNothingMore() async throws {
        let live = try await live("""
        $s = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        $n = 0
        root = Stack([TextContent("n: " + $n), Button("Grow", Action([@Set($n, $n + 1), @Set($s, $s + $s), @Set($n, $n + 100)]))])
        """)
        let window = Window(live)
        defer { window.close() }
        var refused = false
        for _ in 0..<30 {
            let n = try #require(live.value(of: "$n")?.number)
            let s = live.value(of: "$s")
            try window.click("/children/1")
            if live.failure != nil {
                refused = true
                #expect(live.value(of: "$n") == .number(n + 1))
                #expect(live.value(of: "$s") == s)
                #expect(window.texts["/children/0"] == "n: \(JSONValue.format(n + 1))")
                break
            }
        }
        #expect(refused)
    }

    @Test func aRefusedSetThrowsAndChangesNothing() async throws {
        let live = try await live(liveProgram)
        let version = live.version
        let before = texts(drawing: live)
        #expect(throws: (any Error).self) { try live.set("$big", to: .string(String(repeating: "y", count: 9 << 20))) }
        #expect(live.version == version)
        #expect(live.value(of: "$big") == nil)
        #expect(texts(drawing: live) == before)
    }
}

@Suite @MainActor struct WhileAResponseStreams {
    /// The app says a response is streaming: buttons run nothing, a click
    /// already on its way is ignored, inputs can't be typed in, and `run`
    /// refuses. Once it's done, they work again.
    @Test func clicksAreIgnoredAndInputsDisabled() async throws {
        var events: [HostEvent] = []
        let live = try await live(liveProgram) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        live.isStreaming = true
        for event in try window.clickEvents("/children/1") { window.window.sendEvent(event) }
        window.settle()
        try window.click("/children/1")
        #expect(live.value(of: "$hp") == .number(5))
        #expect(events.isEmpty)
        #expect(live.failure == nil)
        #expect(window.fields.isEmpty || window.fields.allSatisfy { !$0.isEnabled })
        #expect(throws: LiveError.self) { try live.run("action", of: try #require(element(live, at: "/children/1")), message: "Hit") }
        #expect(live.value(of: "$hp") == .number(5))

        live.isStreaming = false
        window.settle()
        try window.click("/children/1")
        #expect(live.value(of: "$hp") == .number(4))
        #expect(events.count == 1)
        try window.type("3")
        #expect(live.value(of: "$total") == .number(6))
    }
}

@Suite @MainActor struct TableActions {
    static let program = """
    $label = "Open"
    root = Stack([Table([Col("Go", "action"), Col("Name")], [[$label, "first"]]), TextContent("t")])
    """

    /// Just inside the bottom-left corner of the table: its last row's first cell.
    static func firstCellOfLastRow(_ frame: CGRect) -> CGPoint { CGPoint(x: frame.minX + 8, y: frame.maxY - 6) }

    @Test func anActionCellAsksToContinueWithItsText() async throws {
        var events: [HostEvent] = []
        let live = try await live(Self.program) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try window.click("/children/0", at: Self.firstCellOfLastRow)
        #expect(events.map(\.type) == ["continue_conversation"])
        #expect(events.first?.humanFriendlyMessage == "Open")
        #expect(live.failure == nil)
    }

    /// A click on a cell of a table that has changed since it was drawn runs
    /// nothing; once redrawn, the cell runs with its new text.
    @Test func anActionCellRunsOnlyWhileItsTableIsWhatWasDrawn() async throws {
        var events: [HostEvent] = []
        let live = try await live(Self.program) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try live.set("$label", to: .string("Close"))
        for event in try window.clickEvents("/children/0", at: Self.firstCellOfLastRow) { window.window.sendEvent(event) }
        window.settle()
        #expect(events.isEmpty)
        #expect(live.failure?.contains("changed since this was drawn") == true)
        try window.click("/children/0", at: Self.firstCellOfLastRow)
        #expect(events.map(\.humanFriendlyMessage) == ["Close"])
    }

    @Test func anElementInACellIsDrawnAtItsPathAndRunsItsOwnAction() async throws {
        let live = try await live("""
        $n = 0
        root = Stack([Table([Col("Do")], [[Button("Bump", @Set($n, $n + 1))]]), TextContent("t")])
        """)
        let window = Window(live)
        defer { window.close() }
        try window.click("/children/0/rows/0/0")
        #expect(live.value(of: "$n") == .number(1))
    }
}

@Suite @MainActor struct ASelect {
    /// Picked from the menu its pop-up opens, as a person picks: a bound
    /// select writes its state variable and shows it as the choice.
    @Test func pickingFromItsMenuWritesItsBinding() async throws {
        let live = try await live("""
        $v = "a"
        root = Stack([Select("s", [SelectItem("a", "A"), SelectItem("b", "B")], $v), TextContent("v: " + $v)])
        """, bindings: [("Select", "value")])
        let window = Window(live)
        defer { window.close() }
        let first = try window.pick("B", at: "/children/0")
        #expect(first.map(\.title) == ["s", "A", "B"])
        #expect(first.filter(\.checked).map(\.title) == ["A"])
        #expect(live.value(of: "$v") == .string("b"))
        #expect(window.texts["/children/1"] == "v: b")
        let second = try window.pick(nil, at: "/children/0")
        #expect(second.filter(\.checked).map(\.title) == ["B"])
        #expect(live.value(of: "$v") == .string("b"))
    }

    /// An unbound select in a form writes its form field, with no component
    /// type, as upstream's does.
    @Test func pickingInAFormWritesItsField() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        root = Form("f", [FormControl("U", Select("u", [SelectItem("a", "A"), SelectItem("b", "B")]))], Buttons([Button("Send", @ToAssistant("sent"))]))
        """) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try window.pick("B", at: "/fields/0/input")
        #expect(live.field(form: "f", name: "u") == .string("b"))
        try window.click("/buttons/buttons/0")
        #expect(events.first?.formState == .object([("f", .object([("u", .object([("value", .string("b"))]))]))]))
    }
}

@Suite @MainActor struct AMountedDrawing {
    @Test func redrawsOnASetAndAClickAndOtherwiseStaysStill() async throws {
        let live = try await live("""
        $hp = 5
        root = Stack([TextContent("HP: " + $hp), Button("Hit", @Set($hp, $hp - 1))])
        """)
        let window = Window(live)
        defer { window.close() }
        #expect(window.texts["/children/0"] == "HP: 5")
        try live.set("$hp", to: .number(9))
        window.settle()
        #expect(window.texts["/children/0"] == "HP: 9")
        try window.click("/children/1")
        #expect(window.texts["/children/0"] == "HP: 8")
        let version = live.version
        for _ in 0..<5 { window.settle() }
        #expect(live.version == version)
    }
}
