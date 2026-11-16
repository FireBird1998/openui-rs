// Upstream's form validation, seen where it shows through the session: a
// primary button in a form whose fields break their rules runs nothing and
// sends nothing.
import AppKit
import OpenUINative
import SwiftUI
import Testing

extension NativeUITests {
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
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        #expect(live.value(of: "$sent") == .bool(false))
        #expect(live.failure == nil)
        try await window.click("/buttons/buttons/3")
        #expect(events.isEmpty)

        try live.set("$email", to: .string("ann@example.com"))
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(events.map(\.humanFriendlyMessage) == ["Sent"])
        #expect(live.value(of: "$sent") == .bool(true))
    }

    /// An unbound field's own value is what's checked: typed into its form field.
    @Test func anUnboundFieldIsCheckedByWhatWasTypedInIt() async throws {
        var events: [HostEvent] = []
        let live = try await live(Self.program) { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        try live.set("$email", to: .string("ann@example.com"))
        await window.settle()
        let name = try #require(window.fields.last)
        #expect(window.window.makeFirstResponder(name))
        try #require(name.currentEditor()).insertText("A")
        await window.settle()
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        // Focusing the field again selects what's in it; typing replaces it.
        #expect(window.window.makeFirstResponder(name))
        try #require(name.currentEditor()).insertText("Ann")
        await window.settle()
        #expect(live.field(form: "signup", name: "name") == .string("Ann"))
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
    }

    /// Upstream checks only for a primary button whose plan sends something
    /// away (a message, a custom action, a mutation), or that has no plan.
    @Test func aSecondaryButtonOrAPlanThatOnlySetsStateIsNotChecked() async throws {
        let live = try await live(Self.program)
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/1")
        #expect(live.value(of: "$sent") == .bool(true))
        try live.set("$sent", to: .bool(false))
        await window.settle()
        try await window.click("/buttons/buttons/2")
        #expect(live.value(of: "$sent") == .bool(true))
    }

    @Test func aButtonOutsideAFormIsNotChecked() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        root = Stack([Input("email", null, {required: true}), Button("Send", @ToAssistant("Sent"))])
        """) { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/children/1")
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
        Case(rules: #"["required", "minLength:3"]"#, value: #""ab""#, blocks: false),
        Case(rules: #"["required", "minLength:3"]"#, value: #""abc""#, blocks: false),
        Case(rules: #"{pattern: "^\\d+$"}"#, value: #""١٢""#, blocks: true),
        Case(rules: #"{pattern: "^\\d+$"}"#, value: #""12""#, blocks: false),
        Case(rules: #"{pattern: "^[\\d]+$"}"#, value: #""١٢""#, blocks: true),
        Case(rules: #"{pattern: "^a$"}"#, value: #""a\n""#, blocks: true),
        Case(rules: "{email: true}", value: #""a@b.co\n""#, blocks: true),
        Case(rules: "{url: true}", value: #""http://user@""#, blocks: true),
        Case(rules: "{url: true}", value: #""http://host:99999""#, blocks: true),
        Case(rules: "{url: true}", value: #""\u00a0http://host""#, blocks: true),
        Case(rules: "{url: true}", value: #"" http://user@host:65535 ""#, blocks: false),
        Case(rules: #"["required"]"#, value: #""""#, blocks: false),
        Case(rules: #"{pattern: "^\\D+$"}"#, value: #""١٢""#, blocks: false),
        Case(rules: #"{pattern: "^\\D+$"}"#, value: #""12""#, blocks: true),
        Case(rules: #"{pattern: "^[\\D]+$"}"#, value: #""١٢""#, blocks: false),
        Case(rules: #"{pattern: "^\\$$"}"#, value: #""$""#, blocks: false),
        Case(rules: #"{pattern: "^[$]$"}"#, value: #""$""#, blocks: false),
        Case(rules: #"{pattern: "["}"#, value: #""abc""#, blocks: false),
        Case(rules: #"{pattern: "^\\w+$"}"#, value: #""é""#, blocks: true), // JavaScript uses ASCII word characters.
        Case(rules: "{url: true}", value: #""http:\r\n//host""#, blocks: false),
        Case(rules: "{url: true}", value: #""http://host:65536""#, blocks: true),
        Case(rules: "{unknown: true}", value: #""""#, blocks: false),
    ])
    func eachRule(_ c: Case) async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        $v = \(c.value)
        root = Form("f", [FormControl("V", Input("v", $v, \(c.rules)))], Buttons([Button("Go", @ToAssistant("go"))]))
        """) { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        #expect(events.count == (c.blocks ? 0 : 1))
    }

    /// A select's rules are checked too: once something is picked, it runs.
    @Test func aSelectsRulesAreChecked() async throws {
        var events: [HostEvent] = []
        let live = try await live("""
        root = Form("f", [FormControl("U", Select("u", [SelectItem("a", "A")], null, {required: true}))], Buttons([Button("Go", @ToAssistant("go"))]))
        """) { events.append($0) }
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty)
        try await window.pick("A", at: "/fields/0/input")
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1)
    }
}

}
