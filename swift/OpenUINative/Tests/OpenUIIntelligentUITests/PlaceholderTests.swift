// A program using every one of the 52 components parses against the
// library with no error, and both native views and placeholders draw their children.
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import SwiftUI
import Testing

/// Every component, with its props in order, including both added chart types.
private let everyComponent = #"""
root = Col([layout, words, inputs, media, play, drawing], 2, "stretch")
layout = Row([Box([Grid([Card([Title("A card", "md")], "soft", 3, 4)], 2, 120, 3), Carousel([Text("One"), Text("Two")], 200, 2), Popover("Info", [Caption("Details")]), Divider()], "col", 2, 3, "surface", "lg", true, "50%", 200, 1, "start", "between", false), Spacer()], 2, "start", "between", true)
words = Col([Title("Title", "lg", "ink"), Text(["Mixed ", Badge("tag", "info", "star")], "sm", "muted", "medium", false, false, true, 2), Caption("Caption", "faint"), Label("Label"), Link("Site", "https://example.com"), List(["one", ListItem("two", "second", "2", [Text("child")])], "number", true), Markdown("# Heading\n\n- item"), Quote("Quoted", "Someone"), Code("let x = 1", "swift"), Math("E = mc^2", false)])
$name = ""
$count = 3
$note = ""
$choice = "a"
$level = 50
$done = false
$pace = "Relaxed"
$day = "2026-01-01"
$left = 60
inputs = Form([Input($name, "Name", "text", "Your name", {required: true}), Input($count, "Count", "number"), TextArea($note, "Note", "Write", 4, {maxLength: 200}), Select($choice, ["a", {value: "b", label: "B"}], "Pick", "Choose", {required: true}), SegmentedControl($choice, ["a", "b"], true), Slider($level, 0, 100, 5, "Level", "%", [{value: 0, label: "Low"}]), Checkbox($done, "Done", true, {required: true}), RadioGroup($pace, ["Relaxed", {value: "Packed", label: "Packed", description: "More each day"}], "Pace", "cards"), DatePicker($day, "Day", "2026-01-01", "2026-12-31", {required: true}), Timer($left, "Timer"), Row([Button("Send", @ToAssistant("Send it"), "solid", "primary", "md", "send", false, true), IconButton("plus", @Set($count, $count + 1), "More", "ghost"), Pressable([Text("Tile")], @Set($done, true), $done, 2, "surface", "md")])])
rows = [{month: "Jan", sales: 3, cost: 2}, {month: "Feb", sales: 5, cost: 4}]
media = Col([Image("http://127.0.0.1:1/picture.png", "A photo", 1.5, "lg", "cover"), Icon("map-pin", "md", "info"), Table(["Name", "Qty"], [["Apples", 3], ["Pears", Badge("none")]]), Chart("bar", rows, "month", ["sales", "cost"], "number", true, 200), Chart("radar", rows, "month", ["sales", "cost"]), Chart("radial", rows, "month", ["sales"]), YouTube("dQw4w9WgXcQ", "A video")])
play = Row([Flashcard("Front", [Text("Back")], "Tap"), Celebration($done)])
drawing = Col([Svg([Circle(10, 10, 5, "chart1", "ink", 1, 0.5), Ellipse(30, 10, 8, 4), Rect(0, 20, 20, 10, "surface", "ink", 1, 1, 2), Line(0, 0, 40, 40, "info", 2, 1), Path("M0 0 L10 10", "none"), Polyline("0,0 10,5 20,0"), Polygon("0,0 10,5 20,0"), SvgText("Hi", 5, 50, 12, "ink", "middle", "medium"), G([Circle(50, 50, 3)], "rotate(30 50 50)", 0.5)], "0 0 100 100", "Shapes", 200, "auto"), HtmlView("<p>Hi</p>", 200, "Page")])
"""#

@Suite @MainActor struct PlaceholderTests {
    @Test func everyComponentParsesAndDrawsItsChildren() async throws {
        let core = Core()
        let library = try await core.intelligentUILibrary()
        let step = try await core.parseOneShot(everyComponent, library: library)
        #expect(step.result.meta.errors.map(\.message) == [])
        #expect(step.result.meta.unresolved == [])
        #expect(step.result.meta.incomplete == false)

        let live = try LiveProgram(library: library)
        try await live.show(response: everyComponent, streaming: false)
        #expect(live.errors.map(\.message) == [])
        var drawn: Set<String> = []
        var fallback: [String] = []
        let drawing = ElementDrawing(
            chrome: { view, element, _ in
                drawn.insert(element.typeName)
                return view
            },
            fallback: { element, _ in
                fallback.append(element.typeName)
                return AnyView(EmptyView())
            }
        )
        let view = ElementView(live: live).environment(\.elementDrawing, drawing)
        let image = try #require(ImageRenderer(content: view).nsImage)
        #expect(image.size.width > 0 && image.size.height > 0)
        #expect(drawn == Set(IntelligentUI.components.map(\.name)))
        #expect(fallback == [])
        // A placeholder names its component and holds the components in its props.
        let root = try #require(live.root)
        let form = try #require(root.layoutChildren.first { $0.element?.typeName == "Form" }?.element)
        #expect(Placeholder(element: form).title == "Form")
        #expect(Placeholder.children(of: form).first?.typeName == "Input")
    }
}
