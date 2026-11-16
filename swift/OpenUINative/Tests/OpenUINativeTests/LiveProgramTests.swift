// A live program through the package's public interface: drawn with
// ElementView from a runtime session, clicked and typed in through the
// built-in views as they sit in a window.
import AppKit
import OpenUINative
import SwiftUI
import Testing

/// Components this package draws, and one it doesn't.
let liveSchema = #"""
{"$defs": {
  "Stack": {"properties": {"children": {"type": "array"}}},
  "TextContent": {"properties": {"text": {"type": "string"}}},
  "Button": {"properties": {"label": {"type": "string"}, "action": {}, "variant": {"type": "string"}}},
  "Input": {"properties": {"name": {"type": "string"}, "value": {}, "rules": {}}},
  "Select": {"properties": {"name": {"type": "string"}, "items": {"type": "array"}, "value": {}, "rules": {}}},
  "SelectItem": {"properties": {"value": {"type": "string"}, "label": {"type": "string"}}},
  "Table": {"properties": {"columns": {"type": "array"}, "rows": {"type": "array"}}},
  "Col": {"properties": {"label": {"type": "string"}, "type": {"type": "string"}}},
  "Gauge": {"properties": {"level": {"type": "number"}, "value": {}, "onTap": {}}},
  "Panel": {"properties": {"child": {}}},
  "Form": {"properties": {"name": {"type": "string"}, "fields": {"type": "array"}, "buttons": {}}},
  "FormControl": {"properties": {"label": {"type": "string"}, "input": {}}},
  "Buttons": {"properties": {"buttons": {"type": "array"}}}
}}
"""#

/// State, a button whose action changes it and asks the assistant something,
/// and an input bound to state through a transform.
let liveProgram = """
$hp = 5
$total = 0
root = Stack([hp, hit, amount, total])
hp = TextContent("HP: " + $hp)
hit = Button("Hit", Action([@Set($hp, $hp - 1), @ToAssistant("Ouch")]))
amount = Input("amount", $total = $value * 2)
total = TextContent("Total: " + $total)
"""

@MainActor func live(_ program: String, bindings: [(component: String, prop: String)] = [("Input", "value")],
                     events: @escaping @MainActor (HostEvent) -> Void = { _ in }) async throws -> LiveProgram {
    let core = Core()
    let library = try await core.library(schema: liveSchema)
    let live = try LiveProgram(library: library, bindings: bindings, onHostEvent: events)
    try live.show(try await core.parseOneShot(program, library: library))
    return live
}

/// The texts drawn, as the drawing was asked to draw them: every TextContent's
/// text, by path.
@MainActor func texts(drawing live: LiveProgram) -> [String: String] {
    var drawn: [String: String] = [:]
    let drawing = ElementDrawing(chrome: { view, element, path in
        if element.typeName == "TextContent", case let .scalar(.string(text))? = element.props.first?.value {
            drawn[path] = text
        }
        return view
    })
    let renderer = ImageRenderer(content: ElementView(live: live).environment(\.elementDrawing, drawing))
    renderer.proposedSize = ProposedViewSize(width: 400, height: nil)
    _ = renderer.nsImage
    return drawn
}

/// A live program in a window, as an app shows it, clicked and typed in the
/// way AppKit delivers a person's clicks and keys. The window sits far off
/// screen; SwiftUI only handles clicks in a window that's ordered in.
@MainActor final class Window {
    let window: NSWindow
    let host: NSHostingView<AnyView>
    /// Where each element was drawn, in the window's flipped coordinates, by
    /// key (its path unless the test keys it otherwise).
    private var frames: [String: CGRect] = [:]
    /// The text of every TextContent as the mounted view last drew it, by path.
    private(set) var texts: [String: String] = [:]

    init(_ live: LiveProgram, component: (@MainActor (Element, String) -> AnyView?)? = nil,
         key: @escaping @MainActor (Element, String) -> String = { _, path in path }) {
        host = NSHostingView(rootView: AnyView(EmptyView()))
        window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: 500, height: 400),
                          styleMask: [.borderless], backing: .buffered, defer: false)
        var drawing = ElementDrawing(chrome: { [weak self] view, element, path in
            if element.typeName == "TextContent", case let .scalar(.string(text))? = element.props.first?.value {
                self?.texts[path] = text
            }
            let key = key(element, path)
            return AnyView(view.onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { self?.frames[key] = $0 })
        })
        if let component { drawing.component = component }
        host.rootView = AnyView(ElementView(live: live).environment(\.elementDrawing, drawing))
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        settle()
    }

    func close() {
        window.close()
    }

    /// Lets SwiftUI apply what changed.
    func settle() {
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        host.layoutSubtreeIfNeeded()
    }

    /// The mouse events of `clicks` clicks on what's drawn at `key`: in its
    /// middle, or where `at` says in its frame (flipped: y grows downward).
    func clickEvents(_ key: String, clicks: Int = 1, at: (CGRect) -> CGPoint = { CGPoint(x: $0.midX, y: $0.midY) }) throws -> [NSEvent] {
        let frame = try #require(frames[key], "nothing drawn at \(key)")
        let point = host.convert(at(frame), to: nil)
        var events: [NSEvent] = []
        for _ in 0..<clicks {
            for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                events.append(try #require(NSEvent.mouseEvent(
                    with: type, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1
                )))
            }
        }
        return events
    }

    /// Clicks the middle of what's drawn at `key` (or where `at` says), then
    /// lets it redraw.
    func click(_ key: String, at: (CGRect) -> CGPoint = { CGPoint(x: $0.midX, y: $0.midY) }) throws {
        for event in try clickEvents(key, at: at) { window.sendEvent(event) }
        settle()
    }

    /// Clicks the pop-up drawn at `key`, as a person does, and from the menu
    /// that opens picks the item titled `title` (or, with nil, picks nothing).
    /// Returns the menu's items as it showed them, with whether each was
    /// checked as the current choice.
    @discardableResult
    func pick(_ title: String?, at key: String) throws -> [(title: String, checked: Bool)] {
        var shown: [(title: String, checked: Bool)] = []
        let token = NotificationCenter.default.addObserver(forName: NSMenu.didBeginTrackingNotification, object: nil, queue: nil) { note in
            nonisolated(unsafe) let object = note.object
            MainActor.assumeIsolated {
                guard let menu = object as? NSMenu else { return }
                shown = menu.items.map { ($0.title, $0.state == .on) }
                nonisolated(unsafe) let tracked = menu
                // The menu tracks the mouse in a loop of its own; choose from inside it.
                let choose = Timer(timeInterval: 0.02, repeats: false) { _ in
                    MainActor.assumeIsolated {
                        if let title, let i = tracked.items.firstIndex(where: { $0.title == title }) {
                            tracked.performActionForItem(at: i)
                        }
                        tracked.cancelTracking()
                    }
                }
                RunLoop.main.add(choose, forMode: .common)
            }
        }
        defer { NotificationCenter.default.removeObserver(token) }
        try click(key)
        settle()
        try #require(!shown.isEmpty, "no menu opened at \(key)")
        return shown
    }

    /// Types each of `texts` into the first text field, one after another
    /// with no pass of the run loop between them, then lets it redraw.
    func typeAtOnce(_ texts: [String]) throws {
        let found = try #require(fields.first, "no text field")
        #expect(window.makeFirstResponder(found))
        let editor = try #require(found.currentEditor())
        for text in texts { editor.insertText(text) }
        settle()
    }

    var fields: [NSTextField] {
        func fields(in view: NSView) -> [NSTextField] {
            ((view as? NSTextField).flatMap { $0.isEditable ? [$0] : nil } ?? []) + view.subviews.flatMap(fields(in:))
        }
        return fields(in: host)
    }

    /// Types `text` into the first text field, through its field editor.
    func type(_ text: String) throws {
        let found = try #require(fields.first, "no text field")
        #expect(window.makeFirstResponder(found))
        try #require(found.currentEditor()).insertText(text)
        settle()
    }
}

@Suite @MainActor struct ALiveProgram {
    @Test func drawsExpressionsAsTheirValues() async throws {
        let live = try await live(liveProgram)
        #expect(texts(drawing: live) == ["/children/0": "HP: 5", "/children/3": "Total: 0"])
        #expect(live.errors.isEmpty)
    }

    @Test func settingStateChangesWhatIsDrawn() async throws {
        let live = try await live(liveProgram)
        let before = live.version
        try live.set("$hp", to: .number(9))
        #expect(live.version > before)
        #expect(texts(drawing: live)["/children/0"] == "HP: 9")
        #expect(live.value(of: "$hp") == .number(9))
    }

    @Test func settingTheValueAlreadyHeldChangesNothing() async throws {
        let live = try await live(liveProgram)
        let before = live.version
        try live.set("$hp", to: .number(5))
        #expect(live.version == before)
    }

    @Test func aClickChangesStateAndDeliversAHostEvent() async throws {
        var events: [HostEvent] = []
        let live = try await live(liveProgram) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try window.click("/children/1")
        #expect(live.value(of: "$hp") == .number(4))
        #expect(texts(drawing: live)["/children/0"] == "HP: 4")
        #expect(events.map(\.type) == ["continue_conversation"])
        #expect(events.first?.humanFriendlyMessage == "Ouch")
    }

    @Test func typingInABoundInputWritesThroughItsTransform() async throws {
        let live = try await live(liveProgram)
        let window = Window(live)
        defer { window.close() }
        try window.type("21")
        #expect(live.value(of: "$total") == .number(42))
        #expect(texts(drawing: live)["/children/3"] == "Total: 42")
    }

    @Test func aValueSetSurvivesTheStreamsNextChunk() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library)
        let stream = try await core.session(library: library)
        try live.show(try await stream.push(["$hp = 5\nroot = TextContent(\"HP: \" + $hp)"]))
        try live.set("$hp", to: .number(1))
        try live.show(try await stream.push(["\nother = TextContent(\"more\")"]))
        #expect(texts(drawing: live) == ["/": "HP: 1"])
    }

    @Test func aPropThatFailsKeepsItsExpressionAndIsReported() async throws {
        let live = try await live("""
        $bad = {toString: 1, valueOf: 2}
        root = Stack([TextContent("value: " + $bad), TextContent("fine")])
        """)
        let issue = try #require(live.errors.first)
        #expect(issue.code == "runtime-error")
        #expect(issue.component == "TextContent")
        #expect(issue.message.contains("\"text\""))
        var drawn: [String: PropValue] = [:]
        let drawing = ElementDrawing(chrome: { view, element, path in
            drawn[path] = element.props.first?.value
            return view
        })
        _ = ImageRenderer(content: ElementView(live: live).environment(\.elementDrawing, drawing)).nsImage
        guard case .expression? = drawn["/children/0"] else {
            Issue.record("the failed prop isn't its expression: \(String(describing: drawn["/children/0"]))")
            return
        }
        guard case .scalar(.string("fine"))? = drawn["/children/1"] else {
            Issue.record("the other text didn't evaluate"); return
        }
    }

    @Test func formFieldsAreKeptByFormAndGoWithItsEvents() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        root = Form("signup", [FormControl("Name", Input("name"))], Buttons([Button("Send", @ToAssistant("Sent"))]))
        """) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try window.type("Ann")
        #expect(live.field(form: "signup", name: "name") == .string("Ann"))
        try window.click("/buttons/buttons/0")
        let event = try #require(events.first)
        #expect(event.formName == "signup")
        // Upstream's Input writes its field with no component type.
        #expect(event.formState == .object([("signup", .object([("name", .object([("value", .string("Ann"))]))]))]))
    }

    @Test func aButtonWithNoActionAsksToContinueWithItsLabel() async throws {
        var events: [HostEvent] = []
        let live = try await live(#"root = Stack([TextContent("x"), Button("Tell me more")])"#) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try window.click("/children/1")
        #expect(events.map(\.type) == ["continue_conversation"])
        #expect(events.first?.humanFriendlyMessage == "Tell me more")
    }

    @Test func anOlderActionObjectBecomesOneHostEvent() async throws {
        var events: [HostEvent] = []
        let live = try await live(#"root = Stack([TextContent("x"), Button("Docs", {type: "open_url", url: "https://example.com"})])"#) {
            events.append($0)
        }
        let window = Window(live)
        defer { window.close() }
        try window.click("/children/1")
        #expect(events.map(\.type) == ["open_url"])
        #expect(events.first?.params["url"] == .string("https://example.com"))
    }
}

/// An app's own component, drawn the way the built-in ones are.
@Suite @MainActor struct AnAppsOwnComponent {
    static let program = """
    $hp = 5
    root = Stack([Gauge($hp * 10, $hp, @Set($hp, 0))])
    """

    /// What the app's component was given, by path.
    @MainActor final class Given {
        var props: [String: [Prop]] = [:]
    }

    func draw(_ live: LiveProgram) -> Given {
        let given = Given()
        let drawing = ElementDrawing(component: { element, path in
            guard element.typeName == "Gauge" else { return nil }
            given.props[path] = element.props
            return AnyView(Text("gauge"))
        })
        _ = ImageRenderer(content: ElementView(live: live).environment(\.elementDrawing, drawing)).nsImage
        return given
    }

    @Test func receivesEvaluatedPropsAMarkerAndAPlan() async throws {
        let live = try await live(Self.program, bindings: [("Gauge", "value")])
        let props = try #require(draw(live).props["/children/0"])
        let byName = Dictionary(uniqueKeysWithValues: props.map { ($0.name, $0.value) })
        guard case .scalar(.number(50))? = byName["level"] else { Issue.record("level: \(String(describing: byName["level"]))"); return }
        guard case let .binding(marker)? = byName["value"] else { Issue.record("value: \(String(describing: byName["value"]))"); return }
        #expect(marker.target == "$hp")
        guard case let .plan(plan)? = byName["onTap"] else { Issue.record("onTap: \(String(describing: byName["onTap"]))"); return }
        #expect(plan.steps.count == 1)
    }

    @Test func writesThroughItsMarkerAndRunsItsPlan() async throws {
        let live = try await live(Self.program, bindings: [("Gauge", "value")])
        guard case let .binding(marker)? = draw(live).props["/children/0"]?.first(where: { $0.name == "value" })?.value else {
            Issue.record("no marker"); return
        }
        try live.write(marker, .number(3))
        #expect(draw(live).props["/children/0"]?.first?.value.summary == "30")
        guard case let .element(stack)? = live.root.map(PropValue.element), case let .list(children)? = stack.props.first?.value,
              case let .element(gauge)? = children.first else { Issue.record("no gauge"); return }
        try live.run("onTap", of: gauge, message: "")
        #expect(live.value(of: "$hp") == .number(0))
    }
}
