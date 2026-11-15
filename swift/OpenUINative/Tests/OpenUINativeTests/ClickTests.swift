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

    @Test func anElementFromAnEarlierEvaluationRunsNothing() async throws {
        let live = try await live("""
        $n = 0
        root = Stack([Button("Add", @Set($n, $n + 1))])
        """)
        let old = try #require(element(live, at: "/children/0"))
        #expect(live.isCurrent(old))
        try live.set("$m", to: .number(1))
        #expect(!live.isCurrent(old))
        #expect(throws: LiveError.self) { try live.run("action", of: old, message: "") }
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

    @Test func aMarkerFromAnEarlierEvaluationWritesNothing() async throws {
        let live = try await live("$hp = 5\nroot = Stack([Gauge(1, $hp, null)])", bindings: [("Gauge", "value")])
        guard case let .binding(marker)? = element(live, at: "/children/0")?.props.first(where: { $0.name == "value" })?.value else {
            Issue.record("no marker"); return
        }
        try live.set("$other", to: .number(1))
        #expect(throws: LiveError.self) { try live.write(marker, .number(9)) }
        #expect(live.value(of: "$hp") == .number(5))
    }
}

@Suite @MainActor struct DecodingProps {
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
