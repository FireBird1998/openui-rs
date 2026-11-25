// Text: upstream's lib/{Title,Text,Caption,Label,Badge,Link,List,Markdown,Quote,Code,Math}.
import OpenUINative

extension IntelligentUI {
    static let text: [DefinedComponent] = [
        defineComponent(name: "Title", description: "Heading; md for a card title, lg for a section, 2xl for a big number.", props: [
            .init("text", .string),
            .init("size", .words(["sm", "md", "lg", "xl", "2xl", "3xl"]), optional: true),
            .init("color", .color, optional: true),
        ]) { element, path in TextLeaf(element: element, path: path) },
        defineComponent(name: "Text", description: #"Paragraph; supports **bold**, *italic*, `code` and [links](https://…). text may be a list mixing strings and inline components: [Icon("clock", "sm"), " 25 min · ", Badge("Easy")]."#, props: [
            .init("text", .richText),
            .init("size", .words(["xs", "sm", "md", "lg"]), optional: true),
            .init("color", .color, optional: true),
            .init("weight", .words(["normal", "medium", "semibold"]), optional: true),
            .init("italic", .boolean, optional: true),
            .init("lineThrough", .boolean, optional: true),
            .init("tabularNums", .boolean, optional: true),
            .init("maxLines", .number, optional: true),
        ]) { element, path in TextLeaf(element: element, path: path) },
        defineComponent(name: "Caption", description: "Small secondary text.", props: [
            .init("text", .string),
            .init("color", .color, optional: true),
        ]) { element, path in TextLeaf(element: element, path: path) },
        defineComponent(name: "Label", description: "Field or group label.", props: [
            .init("text", .string),
        ]) { element, path in TextLeaf(element: element, path: path) },
        defineComponent(name: "Badge", description: "Pill tag; icon is an optional leading lucide name.", props: [
            .init("text", .string),
            .init("color", .words(["neutral", "info", "success", "warning", "danger", "purple", "pink"]), optional: true),
            .init("icon", .string, optional: true),
        ]) { element, _ in BadgeDrawing(element: element) },
        defineComponent(name: "Link", description: "Opens an http(s) URL in a new tab.", props: [
            .init("text", .string),
            .init("url", .string),
        ]) { element, path in TextLeaf(element: element, path: path) },
        defineComponent(name: "List", description: "timeline joins the markers with a line.", props: [
            .init("items", .array(.union([.string, .component("ListItem")]))),
            .init("marker", .words(["bullet", "number", "none"]), optional: true),
            .init("timeline", .boolean, optional: true),
        ]) { element, path in ListDrawing(element: element, path: path) },
        defineComponent(name: "ListItem", description: "marker replaces the bullet or number, e.g. a time or an emoji.", props: [
            .init("title", .string),
            .init("description", .string, optional: true),
            .init("marker", .string, optional: true),
            .init("children", .children, optional: true),
        ]) { element, path in ListItemDrawing(element: element, path: path) },
        defineComponent(name: "Markdown", description: "Prose with headings, lists, quotes, code and tables.", props: [
            .init("text", .string),
        ]) { element, _ in MarkdownDrawing(element: element) },
        defineComponent(name: "Quote", description: "A quotation or a callout with a side rule; cite names the source.", props: [
            .init("text", .string),
            .init("cite", .string, optional: true),
        ]) { element, _ in QuoteDrawing(element: element) },
        component("Code", "Code block with a copy button.", [
            .init("code", .string),
            .init("language", .string, optional: true),
        ]),
        component("Math", #"LaTeX formula, e.g. Math("E = mc^2"); inline true to sit in a Text list."#, [
            .init("tex", .string),
            .init("inline", .boolean, optional: true),
        ]),
    ]
}
