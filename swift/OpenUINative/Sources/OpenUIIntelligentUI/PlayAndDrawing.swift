// Play: upstream's lib/{Flashcard,Celebration}. Drawing: lib/Svg (Svg, the
// shapes and G, in upstream's declaration order) and lib/HtmlView.
import OpenUINative

extension IntelligentUI {
    static let play: [DefinedComponent] = [
        component("Flashcard", "Card that flips between front and back when tapped. Put several in a Carousel or Grid.", [
            .init("front", .slot),
            .init("back", .slot),
            .init("hint", .string, optional: true),
        ]),
        component("Celebration", "Bursts confetti when its condition turns true, e.g. Celebration($score == 5) at the end of a quiz.", [
            .init("when", .boolean),
        ]),
    ]

    private static let filledByDefault = "fill defaults to the text color; with only a stroke it stays unfilled."

    static let drawing: [DefinedComponent] = [
        component("Svg", "A drawing for steps, parts, mechanisms, diagrams and layouts. Scales to its width; its shapes take expressions, so state can move them. label says in a few words what it shows, for screen readers: always give one.", [
            .init("children", .drawables),
            .init("viewBox", .string),
            .init("label", .string, optional: true),
            .init("width", .length, optional: true),
            .init("height", .length, optional: true),
        ]),
        component("Circle", filledByDefault, [
            .init("cx", .number), .init("cy", .number), .init("r", .number),
        ] + paint),
        component("Ellipse", filledByDefault, [
            .init("cx", .number), .init("cy", .number), .init("rx", .number), .init("ry", .number),
        ] + paint),
        component("Rect", "rx rounds the corners; fill defaults to the text color, and with only a stroke it stays unfilled.", [
            .init("x", .number), .init("y", .number), .init("width", .number), .init("height", .number),
        ] + paint + [
            .init("rx", .number, optional: true),
        ]),
        component("Line", "Stroked in the text color, 2 wide, unless stroke and strokeWidth are set.", [
            .init("x1", .number), .init("y1", .number), .init("x2", .number), .init("y2", .number),
        ] + strokePaint),
        component("Path", "d is SVG path data only (M, L, C, A, Z...), else nothing draws; unfilled unless fill is set.", [
            .init("d", .string),
        ] + paint),
        component("Polyline", #"points like "0,0 10,5 20,0"."#, [
            .init("points", .string),
        ] + paint),
        component("Polygon", #"Closed shape; points like "0,0 10,5 20,0". fill defaults to the text color; with only a stroke it stays unfilled."#, [
            .init("points", .string),
        ] + paint),
        component("SvgText", "A text label at x, y; anchor sets which end sits on x.", [
            .init("text", .string),
            .init("x", .number),
            .init("y", .number),
            .init("size", .number, optional: true),
            .init("fill", .color, optional: true),
            .init("anchor", .words(["start", "middle", "end"]), optional: true),
            .init("weight", .words(["normal", "medium", "semibold"]), optional: true),
        ]),
        component("G", #"Group; transform like "rotate(30 50 50)" or "translate(10 0)"; one with the wrong number of arguments is dropped."#, [
            .init("children", .shapes),
            .init("transform", .string, optional: true),
            .init("opacity", .number, optional: true),
        ]),
        defineComponent(name: "HtmlView", description: "Sandboxed HTML page with inline JS, for games and canvas. No network except script CDNs. Fits its content unless height is set.", props: [
            .init("html", .string),
            .init("height", .number, optional: true),
            .init("title", .string, optional: true),
        ]) { element, _ in
            HtmlView(element: element)
        },
    ]
}
