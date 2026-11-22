import SwiftUI
import Testing
@testable import OpenUINative

@Suite @MainActor struct ComponentDefinitionTests {
    @Test func everyPropKindProducesPlainDataAndTheCoreSignature() async throws {
        let card = defineComponent(name: "Card", description: "A card.", props: [
            .init("title", .string, binding: true), .init("n", .number, optional: true),
            .init("flag", .boolean), .init("payload", .any), .init("size", .words(["sm", "lg"])),
            .init("items", .array(.component("Item"))), .init("choice", .union([.number, .string])),
            .init("point", .object([.init("y", .number), .init("x", .number, optional: true)])),
            .init("labels", .record(.string)), .init("note", .nullable(.string)),
            .init("go", .action()), .init("open", .action("OpenUrl")), .init("tint", .named("Color", .string))
        ])
        let item = defineComponent(name: "Item", description: "", props: [])
        let groups = [ComponentGroup(name: "Content", components: ["Item", "Card"], notes: ["In order."])]
        let text = try libraryDefinition(components: [card, item], groups: groups, root: "Card")
        let expected = try JSONValue.parse(#"""
        {"components":[{"name":"Card","description":"A card.","props":[
          {"name":"title","type":{"kind":"string"},"binding":true},
          {"name":"n","type":{"kind":"number"},"optional":true},
          {"name":"flag","type":{"kind":"boolean"}},{"name":"payload","type":{"kind":"any"}},
          {"name":"size","type":{"kind":"enum","values":["sm","lg"]}},
          {"name":"items","type":{"kind":"array","items":{"kind":"component","name":"Item"}}},
          {"name":"choice","type":{"kind":"union","options":[{"kind":"number"},{"kind":"string"}]}},
          {"name":"point","type":{"kind":"object","fields":[{"name":"y","type":{"kind":"number"}},
              {"name":"x","type":{"kind":"number"},"optional":true}]}},
          {"name":"labels","type":{"kind":"record","values":{"kind":"string"}}},
          {"name":"note","type":{"kind":"nullable","type":{"kind":"string"}}},
          {"name":"go","type":{"kind":"action"}},{"name":"open","type":{"kind":"action","name":"OpenUrl"}},
          {"name":"tint","type":{"kind":"named","name":"Color","type":{"kind":"string"}}}]},
          {"name":"Item","description":"","props":[]}],
          "componentGroups":[{"name":"Content","components":["Item","Card"],"notes":["In order."]}],"root":"Card"}
        """#)
        #expect(try JSONValue.parse(text) == expected)
        let library = try await Core().createLibrary(components: [card, item], groups: groups, root: "Card")
        let specText = try #require(try await library.promptSpec())
        let spec = try JSONValue.parse(specText)
        let signature = #"Card(title: $binding<string>, n?: number, flag: boolean, payload: any, size: "sm" | "lg", items: Item[], choice: number | string, point: {y: number, x?: number}, labels: Record<string, string>, note?: string, go: ActionExpression, open: @OpenUrl, tint: Color)"#
        #expect(spec["components"]?["Card"]?["signature"] == .string(signature))
        #expect(spec["components"]?["Item"]?["signature"] == .string("Item()"))
        #expect(spec["root"] == expected["root"])
        #expect(spec["componentGroups"] == expected["componentGroups"])
    }

    @Test func liveDrawingUsesTheDefinedViewAndItsEvaluatedProps() async throws {
        var titles: [String] = [], paths: [String] = []
        let badge = defineComponent(name: "Badge", description: "A labeled badge.", props: [.init("title", .string)]) { element, path in
            let title = element.props.first(where: { $0.name == "title" })?.value.summary ?? ""
            let _ = titles.append(title)
            Rectangle().frame(width: 32, height: 12)
        }
        let library = try await Core().createLibrary(components: [badge], root: "Badge")
        let live = try LiveProgram(library: library)
        try await live.show(response: "$v = \"first\"\nroot = Badge($v)", streaming: false)
        let drawing = ElementDrawing(chrome: { view, _, path in
            paths.append(path)
            return view
        })
        let view = ElementView(live: live).environment(\.elementDrawing, drawing)
        #expect(ImageRenderer(content: view).nsImage?.size == CGSize(width: 32, height: 12))
        #expect(titles.last == #""first""#)
        #expect(paths.last == "/")
        try live.set("$v", to: .string("second"))
        #expect(ImageRenderer(content: view).nsImage?.size == CGSize(width: 32, height: 12))
        #expect(titles.last == #""second""#)
    }

    @Test func theDeclaredRootFeedsOneShotMergedAndStreamingParsers() async throws {
        let core = Core()
        let badge = defineComponent(name: "Badge", description: "", props: [])
        let item = defineComponent(name: "Item", description: "", props: [])
        let library = try await core.createLibrary(components: [badge, item], root: "Badge")
        let program = "a = Item()\nb = Badge()"
        let step = try await core.parseOneShot(program, library: library)
        #expect(step.result.root?.typeName == "Badge")
        let merged = try await core.mergeAndParse(base: program, patch: "", library: library, root: nil, chunks: 1)
        #expect(merged.result.root?.typeName == "Badge")
        let live = try LiveProgram(library: library)
        try await live.show(response: program, streaming: false)
        #expect(live.root?.typeName == "Badge")
        let overridden = try await core.parseOneShot(program, library: library, root: "Item")
        #expect(overridden.result.root?.typeName == "Item")
    }

    @Test func aDeclarationWithoutAViewUsesTheAppsFallbackEvenForABuiltinName() async throws {
        let props = [PropDefinition("text", .string)]
        let shadowed = defineComponent(name: "TextContent", description: "", props: props) { _, _ in
            Text("shadowed")
        }
        let viewless = defineComponent(name: "TextContent", description: "", props: props)
        let library = try await Core().createLibrary(components: [shadowed, viewless])
        let live = try LiveProgram(library: library)
        try await live.show(response: #"root = TextContent("native props")"#, streaming: false)
        var fallback: [String] = []
        let drawing = ElementDrawing(fallback: { element, path in
            fallback.append(element.typeName + path)
            return AnyView(Rectangle().frame(width: 17, height: 11))
        })
        let view = ElementView(live: live).environment(\.elementDrawing, drawing)
        #expect(ImageRenderer(content: view).nsImage?.size == CGSize(width: 17, height: 11))
        #expect(fallback.last == "TextContent/")
    }

    @Test func childViewsInheritTheLibraryAndTheAppsDrawingOverride() async throws {
        let child = defineComponent(name: "Child", description: "", props: []) { _, _ in
            Rectangle().frame(width: 20, height: 10)
        }
        let panel = defineComponent(name: "Panel", description: "", props: [.init("child", .component(child.name))]) { element, _ in
            if case let .element(child)? = element.props.first?.value { ElementView(element: child) }
        }
        let library = try await Core().createLibrary(components: [panel, child])
        let live = try LiveProgram(library: library)
        try await live.show(response: "root = Panel(Child())", streaming: false)
        let view = ElementView(live: live)
        #expect(ImageRenderer(content: view).nsImage?.size == CGSize(width: 20, height: 10))
        let drawing = ElementDrawing(component: { element, path in
            guard element.typeName == "Child", path == "/child" else { return nil }
            return AnyView(Rectangle().frame(width: 50, height: 30))
        })
        let overridden = view.environment(\.elementDrawing, drawing)
        #expect(ImageRenderer(content: overridden).nsImage?.size == CGSize(width: 50, height: 30))
    }

    @Test func refusedDefinitionsKeepTheCoreKindAndReason() async throws {
        let core = Core()
        let a = defineComponent(name: "A", description: "", props: [])
        await #expect(throws: OpenUIError(.invalidJSON,
            #"[createLibrary] Root component "B" was not found in components. Available components: A"#)) {
            try await core.createLibrary(components: [a], root: "B")
        }
        // An unknown kind can enter only as plain data. Use the same loading
        // seam as createLibrary, without adding an untyped PropType case.
        let raw = #"{"components":[{"name":"A","description":"","props":[{"name":"d","type":{"kind":"date"}}]}]}"#
        await #expect(throws: OpenUIError(.invalidJSON,
            #"library definition: components[0].props[0].type: type kind "date" is not supported"#)) {
            try await core.run { try Library(core: core, definition: raw, components: [], root: nil) }
        }
        let oversized = defineComponent(name: "A", description: String(repeating: "😀", count: 2_097_152), props: [])
        await #expect(throws: OpenUIError(.limitExceeded, "host input byte limit")) {
            try await core.createLibrary(components: [oversized])
        }
    }

    @Test func typedDefinitionsAdmitTheExactUTF8ByteLimitAndRefuseOneMoreByte() async throws {
        let core = Core()
        let prefix = #"{"components":[{"name":"A","description":""#
        let suffix = #"","props":[]}]}"#
        // This prefix in the description costs six UTF-8 bytes after JSON escaping.
        let label = "😀\n" + String(repeating: "a", count: 8_388_608 - prefix.utf8.count - suffix.utf8.count - 6)
        func component(_ description: String) -> DefinedComponent {
            defineComponent(name: "A", description: description, props: [])
        }
        _ = try await core.createLibrary(components: [component(label)])
        await #expect(throws: OpenUIError(.limitExceeded, "host input byte limit")) {
            try await core.createLibrary(components: [component(label + "a")])
        }
    }

    @Test(arguments: [false, true])
    func aDefinedInputsBindingWritesTheStore(explicitSpec: Bool) async throws {
        var text: Binding<String>?
        let editor = defineComponent(name: "Editor", description: "", props: [.init("value", .string, binding: true)]) { element, _ in
            DefinedTextField(element: element) { text = $0 }
        }
        let library = try await Core().createLibrary(components: [editor])
        let spec = explicitSpec ? try await library.promptSpec() : nil
        let live = try LiveProgram(library: library, promptSpec: spec)
        try await live.show(response: "$v = \"original\"\nroot = Editor($v)", streaming: false)
        _ = ImageRenderer(content: ElementView(live: live)).nsImage
        let input = try #require(text)
        #expect(input.wrappedValue == "original")
        input.wrappedValue = "edited"
        #expect(live.value(of: "$v") == .string("edited"))
    }

    @Test func swiftBindingDocumentExampleDrawsItsBadge() async throws {
        let view = try await badgeView()
        let size = try #require(ImageRenderer(content: view).nsImage?.size)
        #expect(size.width > 0 && size.height > 0)
    }
}

private struct DefinedTextField: View {
    @Environment(\.liveProgram) var live
    let element: Element
    let observe: (Binding<String>) -> Void

    var body: some View {
        if let live, case let .binding(marker)? = element.props.first(where: { $0.name == "value" })?.value {
            let text = Binding(get: { live.value(of: marker.target)?.string ?? "" },
                               set: { try? live.write(marker, .string($0)) })
            let _ = observe(text)
            TextField("Value", text: text)
        }
    }
}

// The example in docs/library-definition.md.
@MainActor private func badgeView() async throws -> ElementView {
    let badge = defineComponent(name: "Badge", description: "A short label.",
                                props: [.init("label", .string)]) { element, _ in
        if case let .scalar(.string(label))? = element.props.first?.value {
            Text(label)
        }
    }
    let library = try await Core().createLibrary(components: [badge], root: badge.name)
    let live = try LiveProgram(library: library)
    try await live.show(response: #"root = Badge("Ready")"#, streaming: false)
    return ElementView(live: live)
}
