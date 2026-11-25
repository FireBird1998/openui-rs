// Media and data: upstream's lib/{Image,Icon,Table,Chart,YouTube}. Chart
// carries this library's only additions (IntelligentUIAdditions).
import OpenUINative

extension IntelligentUI {
    /// Upstream's Chart types.
    static let upstreamChartTypes = ["bar", "line", "area", "pie", "donut", "scatter"]

    /// Upstream's Chart description.
    static let upstreamChartDescription = #"Rows of data; x is the category key (a number key for scatter), series the value keys (default: every number key). format applies to pie and donut only: "percent" shows each slice as a share of the total, "number" its value. stacked stacks bar and area series. height sets the plot height in px; pie and donut ignore it and size themselves."#

    static let mediaAndData: [DefinedComponent] = [
        component("Image", "Photo from a URL the user or a tool gave; never invent a URL. If it fails to load, an empty photo placeholder shows. aspectRatio is 0.2 to 5 (default 1.5).", [
            .init("src", .string),
            .init("alt", .string),
            .init("aspectRatio", .number, optional: true),
            .init("radius", .radius, optional: true),
            .init("fit", .fit, optional: true),
        ]),
        component("Icon", #"Lucide icon by name, e.g. "map-pin"."#, [
            .init("name", .string),
            .init("size", .words(["sm", "md", "lg", "xl"]), optional: true),
            .init("color", .color, optional: true),
        ]),
        component("Table", "Each row lists its cells in column order; a cell may be a component. Number columns align right.", [
            .init("columns", .array(.string)),
            .init("rows", .array(.array(.any))),
        ]),
        defineComponent(name: "Chart", description: upstreamChartDescription + IntelligentUIAdditions.chartDescription, props: [
            .init("type", .words(upstreamChartTypes + IntelligentUIAdditions.chartTypes)),
            .init("data", .array(.record(.union([.string, .number])))),
            .init("x", .string, optional: true),
            .init("series", .array(.string), optional: true),
            .init("format", .words(["number", "percent"]), optional: true),
            .init("stacked", .boolean, optional: true),
            .init("height", .number, optional: true),
        ]) { element, _ in IntelligentChart(element: element) },
        component("YouTube", "Embedded YouTube video by id or URL. Only a video you know exists.", [
            .init("video", .string),
            .init("title", .string, optional: true),
        ]),
    ]
}
