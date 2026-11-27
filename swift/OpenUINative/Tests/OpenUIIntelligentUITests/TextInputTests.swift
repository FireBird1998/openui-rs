import OpenUINative
@testable import OpenUIIntelligentUI
import Testing

@MainActor @Suite struct TextInputTests {
    func program(_ source: String) async throws -> LiveProgram {
        let live = try LiveProgram(library: try await Core().intelligentUILibrary())
        try await live.show(response: source, streaming: false)
        #expect(live.errors.isEmpty)
        return live
    }

    func input(_ live: LiveProgram, at index: Int = 0) throws -> TextInput {
        let root = try #require(live.root)
        let child = try #require(root.layoutChildren.dropFirst(index).first?.element)
        return TextInput(child, path: try #require(child.path))
    }

    @Test func editsWriteBothBindingsAndReadExternalValues() async throws {
        let live = try await program("""
        $name = "Ada"
        $notes = "First"
        root = Col([Input($name), TextArea($notes)])
        """)
        let name = try input(live)
        let notes = try input(live, at: 1)
        #expect(name.text(in: live, form: nil) == "Ada")
        #expect(notes.text(in: live, form: nil) == "First")
        #expect(name.edit("Alan", in: live, form: nil, validation: nil))
        #expect(notes.edit("One\nTwo", in: live, form: nil, validation: nil))
        #expect(live.value(of: "$name") == .string("Alan"))
        #expect(live.value(of: "$notes") == .string("One\nTwo"))
        try live.set("$name", to: .string("Grace"))
        try live.set("$notes", to: .string("Changed"))
        #expect(name.text(in: live, form: nil) == "Grace")
        #expect(notes.text(in: live, form: nil) == "Changed")
    }
}

extension TextInputTests {
    @Test func blockedInputsRejectEditsWithoutChangingState() async throws {
        let live = try await program("""
        $name = "Ada"
        root = Col([Input($name), TextArea("Initial", "Notes")])
        """)
        let name = try input(live)
        let notes = try input(live, at: 1)
        live.isStreaming = true
        let version = live.version
        #expect(!name.isEnabled(in: live))
        #expect(!notes.isEnabled(in: live))
        #expect(!name.edit("Wrong", in: live, form: nil, validation: nil))
        #expect(!notes.edit("Wrong", in: live, form: nil, validation: nil))
        #expect(live.version == version)
        #expect(name.text(in: live, form: nil) == "Ada")
        #expect(notes.text(in: live, form: nil) == "Initial")
        live.isStreaming = false
        #expect(name.isEnabled(in: live))
        #expect(notes.isEnabled(in: live))
        #expect(notes.edit("Right", in: live, form: nil, validation: nil))
        #expect(live.field(form: nil, name: "Notes") == .string("Right"))
    }

    @Test func numberEditsKeepInvalidDraftsLocalAndWriteFiniteNumbersOrNull() async throws {
        let live = try await program("""
        $amount = 12
        root = Col([Input($amount, "Amount", "number")])
        """)
        let amount = try input(live)
        for (text, value): (String, JSONValue?) in [
            ("-", .number(12)), ("1.", .number(1)), ("1,234.50", .number(1234.5)),
            ("0x10000000000000001", .number(18446744073709551616)), ("0b101", .number(5)),
            ("0o14", .number(12)), ("0x1p0", .number(12)), ("1e309", .number(12)),
            // The edit supplies null. Both evaluators read the identity
            // binding's null $value through their nullish scope fallback.
            (",", .number(0)), (" \u{FEFF}", nil),
        ] {
            #expect(amount.edit(text, in: live, form: nil, validation: nil))
            #expect(live.value(of: "$amount") == value, "\(text)")
        }
        #expect(amount.text(in: live, form: nil).isEmpty)
    }

    @Test func unboundNumberClearsToNullAndUsesItsLiteralFallback() async throws {
        let live = try await program(#"root = Col([Input(3, "Count", "number")])"#)
        let input = try input(live)
        #expect(input.edit("", in: live, form: nil, validation: nil))
        #expect(live.field(form: nil, name: "Count") == .null)
        #expect(input.text(in: live, form: nil) == "3")
    }

    @Test func propsSupplyLiteralDefaultsLabelsPlaceholdersAndRoundedRows() async throws {
        let live = try await program(#"""
        root = Col([Input(3,"Count",null,"e.g. 42"), TextArea("a",null,"More"),
          TextArea("",null,null,-9), TextArea("",null,null,4.5), TextArea("",null,null,100)])
        """#)
        let single = try input(live)
        #expect(single.label == "Count")
        #expect(single.placeholder == "e.g. 42")
        #expect(single.text(in: live, form: nil) == "3")
        let area = try input(live, at: 1)
        #expect(area.placeholder == "More")
        #expect(area.text(in: live, form: nil) == "a")
        #expect(try [1, 2, 3, 4].map { try input(live, at: $0).rows } == [3, 2, 5, 20])
    }

    @Test(arguments: ["email", "password"])
    func unknownInputKindIsDiagnosedAndFallsBackToText(_ kind: String) async throws {
        let core = Core()
        let library = try await core.intelligentUILibrary()
        let source = #"root = Col([Input("", "Name", "\#(kind)")])"#
        let result = try await core.parseOneShot(source, library: library).result
        let issue = try #require(result.meta.errors.first)
        #expect(result.meta.errors.count == 1)
        #expect(issue.component == "Input" && issue.path == "/type" && issue.code == "type-mismatch")
        #expect(issue.message.contains(#"one of ["text", "number"]"#) && issue.message.contains(kind))
        let parsed = try #require(result.root?.layoutChildren.first?.element)
        #expect(parsed.prop("type") == nil)

        let live = try LiveProgram(library: library)
        try await live.show(response: source, streaming: false)
        let field = try input(live)
        #expect(!field.numeric)
        #expect(field.edit("007", in: live, form: nil, validation: nil))
        #expect(live.field(form: nil, name: "Name") == .string("007"))
    }

    @Test func formRulesCheckOnBlurAndClearOnlyOnAcceptedEdits() async throws {
        let live = try await program(#"""
        root = Col([form])
        form = Form([email, TextArea("", "Notes", null, null, {required: true, maxLength: 2})])
        email = Input("", "Email", null, null, {required: true, email: true})
        """#)
        let form = try #require(live.root?.layoutChildren.first?.element)
        let validation = FormValidation()
        let email = try #require(form.layoutChildren.first?.element)
        let notes = try #require(form.layoutChildren.dropFirst().first?.element)
        let name = TextInput(email, path: try #require(email.path))
        let area = TextInput(notes, path: try #require(notes.path))
        #expect(name.name == "email")
        #expect(area.name == "Notes")
        #expect(name.isRequired(in: validation))
        #expect(!name.isRequired(in: nil))
        name.validate(in: live, form: "form", validation: nil)
        #expect(validation.errors.isEmpty)
        name.validate(in: live, form: "form", validation: validation)
        area.validate(in: live, form: "form", validation: validation)
        #expect(validation.errors == ["email": "This field is required", "Notes": "This field is required"])
        #expect(name.edit("bad", in: live, form: "form", validation: validation))
        #expect(validation.errors["email"] == nil)
        name.validate(in: live, form: "form", validation: validation)
        #expect(validation.errors["email"] == "Please enter a valid email")
        #expect(live.field(form: "form", name: "email") == .string("bad"))
        let version = live.version
        let overLimit = String(repeating: "\n", count: defaultHostByteLimit / 2)
        #expect(!area.edit(overLimit, in: live, form: "form", validation: validation))
        #expect(live.version == version)
        #expect(area.text(in: live, form: "form").isEmpty)
        #expect(validation.errors["Notes"] == "This field is required")
        #expect(name.edit("ada@example.com", in: live, form: "form", validation: validation))
        #expect(area.edit("🙂a", in: live, form: "form", validation: validation))
        area.validate(in: live, form: "form", validation: validation)
        #expect(validation.errors["Notes"] == "Must be no more than 2 characters")
        #expect(live.field(form: "form", name: "Notes") == .string("🙂a"))
        #expect(area.edit("OK", in: live, form: "form", validation: validation))
        name.validate(in: live, form: "form", validation: validation)
        area.validate(in: live, form: "form", validation: validation)
        #expect(validation.errors.isEmpty)
    }

    @Test func fieldsInAnInvalidSettledBundleAreAlsoBlocked() async throws {
        let source = #"""
        $name = "Original"
        root = Col([Input($name), TextArea("", "Notes")])
        """#
        let live = try await program(source)
        let bundle = "]]>openui:content?library=bench\n" + source + "\n]]>openui:scripts {\"count\":1}\n["
        try await live.show(response: bundle, streaming: false)
        #expect(live.isBlocked && !live.isStreaming)
        let version = live.version
        for index in 0..<2 {
            let field = try input(live, at: index)
            #expect(!field.isEnabled(in: live))
            #expect(!field.edit("ignored", in: live, form: nil, validation: nil))
        }
        #expect(live.version == version)
    }

    @Test func inlineFormAndFieldUseTheirOwnPathsAndRejectStaleEdits() async throws {
        let live = try await program(#"root = Col([Form([Input("one"), TextArea("two")])])"#)
        let form = try #require(live.root?.layoutChildren.first?.element)
        let fields = try form.layoutChildren.map { child in
            let element = try #require(child.element)
            return TextInput(element, path: try #require(element.path))
        }
        let scope = try #require(IntelligentFormScope.name(form))
        #expect(scope == "/children/0")
        for field in fields {
            #expect(field.edit(field.name, in: live, form: scope, validation: nil))
            #expect(live.field(form: scope, name: field.name) == .string(field.name))
        }
        try await live.show(response: #"root = Col([Form([Input("new")])])"#, streaming: false)
        let version = live.version
        for field in fields {
            #expect(!field.edit("stale", in: live, form: scope, validation: nil))
        }
        #expect(live.version == version)
    }
}
