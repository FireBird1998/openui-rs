// Compiled only by compare-native-rendering.py into isolated package copies.
import AppKit
import Foundation
import OpenUINative
import SwiftUI

private struct Sample: Decodable {
    var name: String
    var schema: String
    var program: String
    var root: String?
    var spec: String?
    var source: String
}
private struct Node {
    var element: Element
    var path: String
    var form: String?
}
private struct NodeResult: Encodable {
    var path: String
    var type: String
    var native: Bool
    var props: [String]
    var text: [String: Int]
}
private struct Result: Encodable {
    var name: String
    var mode: String
    var source: String
    var nodes: [NodeResult]
    var error: String?
}

@MainActor private func nodes(_ root: Element) -> [Node] {
    var found: [Node] = []
    func visit(_ element: Element, path: String, form: String?) {
        let path = element.path ?? path
        found.append(Node(element: element, path: path, form: form))
        var enclosing = form
        if element.typeName == "Form", let prop = element.props.first(where: { $0.name == "name" }),
           case let .scalar(.string(name)) = prop.value { enclosing = name }
        for prop in element.props { walk(prop.value, path: ElementPath.child(of: path, prop.name), form: enclosing) }
    }
    func walk(_ value: PropValue, path: String, form: String?) {
        switch value {
        case let .element(element): visit(element, path: path, form: form)
        case let .list(items):
            for (i, item) in items.enumerated() { walk(item, path: ElementPath.child(of: path, String(i)), form: form) }
        case let .record(fields):
            for field in fields { walk(field.value, path: ElementPath.child(of: path, field.name), form: form) }
        default: break
        }
    }
    visit(root, path: ElementPath.root, form: nil)
    return found
}

@MainActor private func render(_ node: Node, live: LiveProgram?) -> NodeResult {
    let native = NativeComponents.view(for: node.element, path: node.path) != nil
    let texts = autoreleasepool {
        RenderingAudit.capture {
            // Mount each component with its real props/state/form. Descendant
            // elements are probed separately, including hidden tabs/options.
            // This avoids counting a child repeatedly through every ancestor
            // and avoids mistaking lazy offscreen realization for omission.
            let drawing = ElementDrawing(
                component: { _, path in path == node.path ? nil : AnyView(EmptyView()) },
                fallback: { element, path in AnyView(ElementBox(element: element, path: path)) }
            )
            var content = AnyView(ElementView(element: node.element, path: node.path)
                .environment(\.elementDrawing, drawing).environment(\.formName, node.form))
            if let live { content = AnyView(content.environment(\.liveProgram, live)) }
            let renderer = ImageRenderer(content: content
                .frame(width: 800, height: 600, alignment: .topLeading).clipped()
                .environment(\.colorScheme, .light))
            renderer.proposedSize = ProposedViewSize(width: 800, height: 600)
            renderer.scale = 1
            _ = renderer.nsImage
        }
    }
    return NodeResult(path: node.path, type: node.element.typeName, native: native,
                      props: node.element.props.map(\.name), text: texts)
}

@main struct RenderAudit {
    @MainActor static func main() async throws {
        _ = NSApplication.shared
        guard CommandLine.arguments.count == 3 else { throw CocoaError(.fileReadInvalidFileName) }
        let samples = try JSONDecoder().decode([Sample].self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))
        let output = URL(fileURLWithPath: CommandLine.arguments[2])
        FileManager.default.createFile(atPath: output.path, contents: nil)
        let file = try FileHandle(forWritingTo: output)
        defer { try? file.close() }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        let core = Core()
        var libraries: [String: Library] = [:]
        var count = 0
        for sample in samples {
            var results: [Result] = []
            do {
                let library: Library
                if let cached = libraries[sample.schema] { library = cached }
                else { library = try await core.library(schema: sample.schema); libraries[sample.schema] = library }
                let step = try await core.parseOneShot(sample.program, library: library, root: sample.root)
                results.append(Result(name: sample.name, mode: "parsed", source: sample.source,
                    nodes: step.result.root.map { nodes($0).map { render($0, live: nil) } } ?? []))
                do {
                    let live = try LiveProgram(library: library, promptSpec: sample.spec)
                    try live.show(step)
                    results.append(Result(name: sample.name, mode: "evaluated", source: sample.source,
                        nodes: live.root.map { nodes($0).map { render($0, live: live) } } ?? []))
                } catch {
                    results.append(Result(name: sample.name, mode: "evaluated", source: sample.source, nodes: [], error: String(describing: error)))
                }
            } catch {
                for mode in ["parsed", "evaluated"] {
                    results.append(Result(name: sample.name, mode: mode, source: sample.source, nodes: [], error: String(describing: error)))
                }
            }
            for result in results { try file.write(contentsOf: encoder.encode(result)); try file.write(contentsOf: Data([10])) }
            count += 1
            if count.isMultiple(of: 100) { print("audited \(count)/\(samples.count) programs") }
        }
        print("audited \(count) programs in parsed and evaluated modes")
    }
}
