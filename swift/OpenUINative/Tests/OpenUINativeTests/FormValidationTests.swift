// Upstream's form validation, seen where it shows through the session: a
// primary button in a form whose fields break their rules runs nothing and
// sends nothing.
import AppKit
import OpenUINative
import SwiftUI
import Testing

@Suite @MainActor struct CheckingAForm {
    static let program = """
    $sent = false
    $email = ""
    m = Mutation("t")
    root = Form("signup", [FormControl("Email", Input("email", $email, {required: true, email: true})), FormControl("Name", Input("name", null, {minLength: 2}))], Buttons([Button("Send", Action([@Set($sent, true), @ToAssistant("Sent")])), Button("Save", @Set($sent, true), "secondary"), Button("Mark", @Set($sent, true)), Button("Run", @Run(m))]))
    """

    @Test func aPrimaryButtonRunsNothingWhileAFieldBreaksItsRules() async throws {
        var events: [HostEvent] = []
        let live = try await live(Self.program) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        #expect(live.value(of: "$sent") == .bool(false))
        #expect(live.failure == nil)
        try window.click("/buttons/buttons/3")
        #expect(events.isEmpty)

        try live.set("$email", to: .string("ann@example.com"))
        window.settle()
        try window.click("/buttons/buttons/0")
        #expect(events.map(\.humanFriendlyMessage) == ["Sent"])
        #expect(live.value(of: "$sent") == .bool(true))
    }

    /// An unbound field's own value is what's checked: typed into its form field.
    @Test func anUnboundFieldIsCheckedByWhatWasTypedInIt() async throws {
        var events: [HostEvent] = []
        let live = try await live(Self.program) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try live.set("$email", to: .string("ann@example.com"))
        window.settle()
        let name = try #require(window.fields.last)
        #expect(window.window.makeFirstResponder(name))
        try #require(name.currentEditor()).insertText("A")
        window.settle()
        try window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        // Focusing the field again selects what's in it; typing replaces it.
        #expect(window.window.makeFirstResponder(name))
        try #require(name.currentEditor()).insertText("Ann")
        window.settle()
        #expect(live.field(form: "signup", name: "name") == .string("Ann"))
        try window.click("/buttons/buttons/0")
        #expect(events.count == 1)
    }

    /// Upstream checks only for a primary button whose plan sends something
    /// away (a message, a custom action, a mutation), or that has no plan.
    @Test func aSecondaryButtonOrAPlanThatOnlySetsStateIsNotChecked() async throws {
        let live = try await live(Self.program)
        let window = Window(live)
        defer { window.close() }
        try window.click("/buttons/buttons/1")
        #expect(live.value(of: "$sent") == .bool(true))
        try live.set("$sent", to: .bool(false))
        window.settle()
        try window.click("/buttons/buttons/2")
        #expect(live.value(of: "$sent") == .bool(true))
    }

    @Test func aButtonOutsideAFormIsNotChecked() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        root = Stack([Input("email", null, {required: true}), Button("Send", @ToAssistant("Sent"))])
        """) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try window.click("/children/1")
        #expect(events.count == 1)
    }

    struct Case: CustomTestStringConvertible, Sendable {
        var rules: String
        var value: String
        var blocks: Bool
        var testDescription: String { "\(rules) with \(value) \(blocks ? "blocks" : "runs")" }
    }

    /// Upstream's rules, each kept or broken.
    @Test(arguments: [
        Case(rules: "{required: true}", value: #""""#, blocks: true),
        Case(rules: "{required: true}", value: #""x""#, blocks: false),
        Case(rules: "{required: false}", value: #""""#, blocks: false),
        Case(rules: "{email: true}", value: #""a@b""#, blocks: true),
        Case(rules: "{email: true}", value: #""a@b.co""#, blocks: false),
        Case(rules: "{email: true}", value: #""""#, blocks: false),
        Case(rules: "{url: true}", value: #""example.com""#, blocks: true),
        Case(rules: "{url: true}", value: #""https://example.com/a""#, blocks: false),
        Case(rules: "{numeric: true}", value: #""12abc""#, blocks: false),
        Case(rules: "{numeric: true}", value: #""abc""#, blocks: true),
        Case(rules: "{min: 5}", value: #""4""#, blocks: true),
        Case(rules: "{min: 5}", value: "5", blocks: false),
        Case(rules: "{max: 5}", value: #""6""#, blocks: true),
        Case(rules: "{minLength: 3}", value: #""ab""#, blocks: true),
        Case(rules: "{maxLength: 3}", value: #""abcd""#, blocks: true),
        Case(rules: "{maxLength: 3}", value: #""abc""#, blocks: false),
        Case(rules: #"{pattern: "^[a-z]+$"}"#, value: #""abc1""#, blocks: true),
        Case(rules: #"{pattern: "^[a-z]+$"}"#, value: #""abc""#, blocks: false),
        Case(rules: #"["required", "minLength:3"]"#, value: #""ab""#, blocks: true),
        Case(rules: #"["required", "minLength:3"]"#, value: #""abc""#, blocks: false),
        Case(rules: "{unknown: true}", value: #""""#, blocks: false),
    ])
    func eachRule(_ c: Case) async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        $v = \(c.value)
        root = Form("f", [FormControl("V", Input("v", $v, \(c.rules)))], Buttons([Button("Go", @ToAssistant("go"))]))
        """) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try window.click("/buttons/buttons/0")
        #expect(events.count == (c.blocks ? 0 : 1))
    }

    /// A select's rules are checked too: once something is picked, it runs.
    @Test func aSelectsRulesAreChecked() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        root = Form("f", [FormControl("U", Select("u", [SelectItem("a", "A")], null, {required: true}))], Buttons([Button("Go", @ToAssistant("go"))]))
        """) { events.append($0) }
        let window = Window(live)
        defer { window.close() }
        try window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        try window.pick("A", at: "/fields/0/input")
        try window.click("/buttons/buttons/0")
        #expect(events.count == 1)
    }
}
