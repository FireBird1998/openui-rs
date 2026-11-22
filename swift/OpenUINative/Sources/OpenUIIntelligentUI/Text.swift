// Text: upstream's lib/{Title,Text,Caption,Label,Badge,Link,List,Markdown,Quote,Code,Math}.
import OpenUINative

extension IntelligentUI {
    static let text: [DefinedComponent] = [
        component("Title", "Heading; md for a card title, lg for a section, 2xl for a big number.", [
            .init("text", .string),
            .init("size", .words(["sm", "md", "lg", "xl", "2xl", "3xl"]), optional: true),
            .init("color", .color, optional: true),
        ]),
        component("Text", #"Paragraph; supports **bold**, *italic*, `code` and [links](https://…). text may be a list mixing strings and inline components: [Icon("clock", "sm"), " 25 min · ", Badge("Easy")]."#, [
            .init("text", .richText),
            .init("size", .words(["xs", "sm", "md", "lg"]), optional: true),
            .init("color", .color, optional: true),
            .init("weight", .words(["normal", "medium", "semibold"]), optional: true),
            .init("italic", .boolean, optional: true),
            .init("lineThrough", .boolean, optional: true),
            .init("tabularNums", .boolean, optional: true),
            .init("maxLines", .number, optional: true),
        ]),
        component("Caption", "Small secondary text.", [
            .init("text", .string),
            .init("color", .color, optional: true),
        ]),
        component("Label", "Field or group label.", [
            .init("text", .string),
        ]),
        component("Badge", "Pill tag; icon is an optional leading lucide name.", [
            .init("text", .string),
            .init("color", .words(["neutral", "info", "success", "warning", "danger", "purple", "pink"]), optional: true),
            .init("icon", .string, optional: true),
        ]),
        component("Link", "Opens an http(s) URL in a new tab.", [
            .init("text", .string),
            .init("url", .string),
        ]),
        component("List", "timeline joins the markers with a line.", [
            .init("items", .array(.union([.string, .component("ListItem")]))),
            .init("marker", .words(["bullet", "number", "none"]), optional: true),
            .init("timeline", .boolean, optional: true),
        ]),
        component("ListItem", "marker replaces the bullet or number, e.g. a time or an emoji.", [
            .init("title", .string),
            .init("description", .string, optional: true),
            .init("marker", .string, optional: true),
            .init("children", .children, optional: true),
        ]),
        component("Markdown", "Prose with headings, lists, quotes, code and tables.", [
            .init("text", .string),
        ]),
        component("Quote", "A quotation or a callout with a side rule; cite names the source.", [
            .init("text", .string),
            .init("cite", .string, optional: true),
        ]),
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
