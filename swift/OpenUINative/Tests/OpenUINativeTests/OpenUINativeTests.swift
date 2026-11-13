// The package through its public interface: what an app outside this
// repository can do with it.
import OpenUINative
import SwiftUI
import Testing

/// A library with one component this package draws and one it has never heard of.
let schema = #"""
{"$defs": {
  "Stack": {"properties": {"children": {"type": "array"}}},
  "Dice": {"properties": {"sides": {"type": "number"}}}
}}
"""#

/// What a drawing was asked to draw, by component.
@MainActor final class Asked {
    var components: [String] = []
    var fallbacks: [String] = []
    var chromed: [String] = []
}

@MainActor func root(of program: String) async throws -> Element {
    let core = Core()
    let library = try await core.library(schema: schema)
    return try #require(try await core.parseOneShot(program, library: library).result.root)
}

@MainActor func draw(_ element: Element, with drawing: ElementDrawing) -> Bool {
    let renderer = ImageRenderer(content: ElementView(element: element).environment(\.elementDrawing, drawing))
    renderer.proposedSize = ProposedViewSize(width: 400, height: nil)
    return renderer.nsImage != nil
}

@Suite @MainActor struct AnAppDrawing {
    @Test func drawsAProgramWithNothingSetUp() async throws {
        let stack = try await root(of: #"root = Stack(["hello"])"#)
        #expect(draw(stack, with: ElementDrawing()))
    }

    @Test func isAskedForItsOwnComponentsFirst() async throws {
        let stack = try await root(of: "root = Stack([Dice(6), Stack([])])")
        let asked = Asked()
        let drawing = ElementDrawing(
            component: { element, _ in
                asked.components.append(element.typeName)
                return element.typeName == "Dice" ? AnyView(Text("⚅")) : nil
            },
            fallback: { element, _ in
                asked.fallbacks.append(element.typeName)
                return AnyView(EmptyView())
            }
        )
        #expect(draw(stack, with: drawing))
        #expect(Set(asked.components) == ["Stack", "Dice"])
        #expect(asked.fallbacks.isEmpty)
    }

    @Test func drawsAComponentNobodyKnowsWithTheFallback() async throws {
        let stack = try await root(of: "root = Stack([Dice(6)])")
        let asked = Asked()
        let drawing = ElementDrawing(fallback: { element, path in
            asked.fallbacks.append("\(element.typeName) at \(path)")
            return AnyView(Text("?"))
        })
        #expect(draw(stack, with: drawing))
        #expect(Set(asked.fallbacks) == ["Dice at /children/0"])
    }

    @Test func wrapsWhatItDrawsButNotTheFallback() async throws {
        let stack = try await root(of: "root = Stack([Dice(6), Stack([])])")
        let asked = Asked()
        let drawing = ElementDrawing(chrome: { view, element, path in
            asked.chromed.append("\(element.typeName) at \(path)")
            return view
        })
        #expect(draw(stack, with: drawing))
        #expect(Set(asked.chromed) == ["Stack at /", "Stack at /children/1"])
    }
}
