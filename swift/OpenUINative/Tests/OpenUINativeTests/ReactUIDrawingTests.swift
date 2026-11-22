import Foundation
@testable import OpenUINative
import SwiftUI
import Testing

private func read(_ file: String) throws -> String {
    var repository = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { repository.deleteLastPathComponent() }
    return try String(contentsOf: repository.appendingPathComponent(file), encoding: .utf8)
}

private func importedLibrary(_ schema: String) async throws -> Library {
    try await Core().library(schema: read("conformance/imported/schemas/\(schema).json"))
}

@Suite @MainActor struct ReactUIDrawingTests {
    @Test(arguments: ["react-ui", "bench"])
    func tableDrawsElementCellsAtTheirSourcePaths(_ schema: String) async throws {
        let react = schema == "react-ui"
        let source = react
            ? #"root = Table([Col("Short", [Tag("A")]), Col("Long", [Tag("B"), Tag("C")])])"#
            : #"root = Table([Col("Short"), Col("Long")], [[Tag("A"), Tag("B")], [null, Tag("C")]])"#
        let step = try await Core().parseOneShot(source, library: importedLibrary(schema))
        let root = try #require(step.result.root)
        var paths = Set<String>()
        var placeholders = Set<String>()
        let drawing = ElementDrawing(chrome: { view, element, path in
            if element.typeName == "Tag" { paths.insert(path) }
            return view
        }, fallback: { element, _ in
            placeholders.insert(element.typeName)
            return AnyView(EmptyView())
        })
        let view = ElementView(element: root).environment(\.elementDrawing, drawing)
        _ = ImageRenderer(content: view).cgImage
        #expect(placeholders.isEmpty)
        #expect(paths == (react
            ? ["/columns/0/data/0", "/columns/1/data/0", "/columns/1/data/1"]
            : ["/rows/0/0", "/rows/0/1", "/rows/1/1"]))
    }

    private struct Manifest: Decodable {
        struct Case: Decodable {
            let schema: String
            let file: String
            let root: String?
        }
        let cases: [Case]
    }

    private let parts = [
        "Table.columns": "Col", "BarChart.series": "Series", "LineChart.series": "Series",
        "Tabs.items": "TabItem", "Select.items": "SelectItem", "CheckBoxGroup.items": "CheckBoxItem",
        "RadioGroup.items": "RadioItem", "Chips.items": "ChipItem", "OptionCards.items": "OptionCard",
    ]

    // Include inactive tabs and offscreen cells. Each entry in parts is
    // drawn directly by its parent instead of going through ElementView.
    private func fullyNative(_ root: Element) -> Bool {
        var pending: [(PropValue, String?)] = [(.element(root), nil)]
        while let (value, part) = pending.popLast() {
            switch value {
            case let .element(element):
                if element.typeName != part,
                   NativeComponents.view(for: element, path: element.path ?? "/") == nil { return false }
                for prop in element.props {
                    pending.append((prop.value, parts[element.typeName + "." + prop.name]))
                }
            case let .list(items):
                for item in items { pending.append((item, part)) }
            case let .record(fields):
                for field in fields { pending.append((field.value, nil)) }
            default: break
            }
        }
        return true
    }

    @Test func recordedReactUIProgramsDrawWithoutPlaceholders() async throws {
        let manifest = try JSONDecoder().decode(Manifest.self, from: Data(read("conformance/recorded.json").utf8))
        let programs = manifest.cases.filter { $0.schema == "react-ui" }
        let library = try await importedLibrary("react-ui")
        let spec = try read("conformance/prompts/react-ui-schema.json")
        let core = Core()
        var native = 0
        for program in programs {
            let step = try await core.parseOneShot(read("conformance/" + program.file), library: library, root: program.root)
            let live = try LiveProgram(library: library, promptSpec: spec)
            try live.show(step)
            guard let root = live.root, fullyNative(root) else { continue }
            let asked = Asked()
            let view = ElementView(live: live).environment(\.elementDrawing, ElementDrawing(fallback: { element, _ in
                asked.fallbacks.append(element.typeName)
                return AnyView(EmptyView())
            }))
            let renderer = ImageRenderer(content: view)
            renderer.proposedSize = ProposedViewSize(width: 640, height: nil)
            try #require(renderer.cgImage != nil, "Empty drawing for \(program.file)")
            #expect(asked.fallbacks.isEmpty, "Unexpected placeholder in \(program.file)")
            if asked.fallbacks.isEmpty { native += 1 }
        }
        print("Recorded react-ui programs drawing without a placeholder: \(native)/\(programs.count)")
        #expect(programs.count == 152)
        #expect(native == 18)
    }

    @Test(arguments: ["react-ui", "bench"])
    func pieDrawsOrderedCategoriesAndValuesAsADonut(_ schema: String) async throws {
        let source = schema == "react-ui"
            ? #"root = PieChart(["Peach", "Plum", "Pear"], [7, 3], "donut")"#
            : #"root = PieChart([Slice("Peach", 7), Slice("Plum", 3), Slice("Pear", 0)], "donut")"#
        let step = try await Core().parseOneShot(source, library: importedLibrary(schema))
        let root = try #require(step.result.root)
        let asked = Asked()
        #expect(draw(root, with: ElementDrawing(chrome: { view, element, _ in
            asked.chromed.append(element.typeName)
            return view
        })))
        #expect(asked.chromed.contains("PieChart"))
        let chart = try PieChartView(PropReader(element: root))
        #expect(chart.slices.map(\.series) == ["Peach", "Plum", "Pear"])
        #expect(chart.slices.map(\.value) == [7, 3, 0])
        #expect(chart.donut)
    }

    @Test func everyDocumentedUnsupportedComponentDrawsItsOwnName() throws {
        let doc = try read("swift/OpenUINative/README.md")
        let heading = try #require(doc.range(of: "## Components drawn as placeholders\n"))
        let nextHeading = doc.range(of: "\n## ", range: heading.upperBound..<doc.endIndex)
        let section = doc[heading.upperBound..<(nextHeading?.lowerBound ?? doc.endIndex)]
        let names = Set(section.split(separator: "`", omittingEmptySubsequences: false).enumerated()
            .compactMap { $0.offset % 2 == 1 ? String($0.element) : nil })
        let schema = try JSONValue.parse(read("conformance/imported/schemas/react-ui.json"))
        guard case let .object(components)? = schema["properties"] else {
            Issue.record("Missing component inventory")
            return
        }
        #expect(names == Set(components.map(\.key)).subtracting(NativeComponents.names).subtracting(parts.values))
        for name in names.sorted() {
            let root = Element(typeName: name, props: [], partial: true, hasDynamicProps: false)
            let label = Label(name, systemImage: "questionmark.square.dashed")
                .font(.callout.monospaced()).foregroundStyle(.secondary)
            let expected = try #require(ImageRenderer(content: label).cgImage?.dataProvider?.data)
            let actual = try #require(ImageRenderer(content: ElementView(element: root)).cgImage?.dataProvider?.data)
            #expect(actual as Data == expected as Data, "Placeholder for \(name)")
        }
    }

    @Test func raggedColumnDataDrawsTheSameGridAsExplicitRows() async throws {
        func pixels(_ source: String, _ schema: String) async throws -> Data {
            let step = try await Core().parseOneShot(source, library: importedLibrary(schema))
            let element = try #require(step.result.root)
            let view = ElementView(element: element)
            return try #require(ImageRenderer(content: view).cgImage?.dataProvider?.data) as Data
        }
        let columns = #"Table([Col("Fruit", ["Peach", "Plum"]), Col("Count", [23, 5, 8], "number"), Col("Note", "sold")])"#
        let rows = #"Table([Col("Fruit"), Col("Count", "number"), Col("Note")], [["Peach",23,"sold"],["Plum",5,null],[null,8,null]])"#
        let expected = try await pixels("root = " + rows, "bench")
        let actual = try await pixels("root = " + columns, "react-ui")
        #expect(actual == expected)
    }
}

extension NativeUITests {
    @Suite @MainActor struct ReactUITablePages {
        private struct PageButton: PrimitiveButtonStyle {
            @Environment(\.isEnabled) private var enabled
            let observe: (@escaping () -> Void) -> Void

            func makeBody(configuration: Configuration) -> some View {
                if enabled { observe { configuration.trigger() } }
                return Button(configuration).buttonStyle(.bordered)
            }
        }

        @Test func lastPageKeepsItsNestedButtonsAction() async throws {
            let library = try await importedLibrary("react-ui")
            let live = try LiveProgram(library: library)
            let cells = Array(repeating: #""first page""#, count: 10) + [#"Button("Pick", @Set($picked, 1))"#]
            try await live.show(response: "$picked = 0\nroot = Table([Col(\"Choice\", [\(cells.joined(separator: ","))])])", streaming: false)
            // On the first page the only enabled button is Next page.
            var next: (() -> Void)?
            let window = await Window(live, component: { element, path in
                guard element.typeName == "Table", let table = NativeComponents.view(for: element, path: path) else { return nil }
                return AnyView(table.buttonStyle(PageButton { next = $0 }))
            })
            defer { window.close() }
            let goToNextPage = try #require(next)
            goToNextPage()
            await window.settle()
            try await window.click("/columns/0/data/10")
            #expect(live.value(of: "$picked") == .number(1))
        }
    }
}
