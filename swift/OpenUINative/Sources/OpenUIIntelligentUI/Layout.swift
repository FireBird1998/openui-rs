// Layout: upstream's lib/{Col,Row,Box,Grid,Card,Carousel,Popover,Divider,Spacer}.
import OpenUINative

extension IntelligentUI {
    static let layout: [DefinedComponent] = [
        defineComponent(name: "Col", description: "Vertical layout. Without gap it spaces blocks like an article.", props: [
            .init("children", .children),
            .init("gap", .number, optional: true),
            .init("align", .align, optional: true),
        ]) { element, path in LayoutContainer(element: element, path: path) },
        defineComponent(name: "Row", description: "Horizontal layout; items are vertically centered.", props: [
            .init("children", .children),
            .init("gap", .number, optional: true),
            .init("align", .align, optional: true),
            .init("justify", .justify, optional: true),
            .init("wrap", .boolean, optional: true),
        ]) { element, path in LayoutContainer(element: element, path: path) },
        defineComponent(name: "Box", description: "Flex container, a column by default. radius also clips the content.", props: [
            .init("children", .children, optional: true),
            .init("direction", .words(["row", "col"]), optional: true),
            .init("gap", .number, optional: true),
            .init("padding", .number, optional: true),
            .init("background", .color, optional: true),
            .init("radius", .radius, optional: true),
            .init("border", .boolean, optional: true),
            .init("width", .length, optional: true),
            .init("height", .length, optional: true),
            .init("flex", .number, optional: true),
            .init("align", .align, optional: true),
            .init("justify", .justify, optional: true),
            .init("wrap", .boolean, optional: true),
        ]) { element, path in LayoutContainer(element: element, path: path) },
        component("Grid", "Equal columns, 1 to 12 (default 2). With minChildWidth (px), columns drop on narrow screens.", [
            .init("children", .children),
            .init("columns", .number, optional: true),
            .init("minChildWidth", .number, optional: true),
            .init("gap", .number, optional: true),
        ]),
        defineComponent(name: "Card", description: "Panel with a border; soft is a filled panel for summaries.", props: [
            .init("children", .children),
            .init("variant", .words(["outline", "soft"]), optional: true),
            .init("gap", .number, optional: true),
            .init("padding", .number, optional: true),
        ]) { element, path in LayoutContainer(element: element, path: path) },
        component("Carousel", "Horizontal row that scrolls and snaps, each child an item itemWidth px wide (default 240): photos, cards, options, days.", [
            .init("children", .children),
            .init("itemWidth", .number, optional: true),
            .init("gap", .number, optional: true),
        ]),
        component("Popover", #"Shows content in a small panel when the trigger is clicked: a definition, a source, details behind an Icon("info", "sm"). The trigger is text or an Icon, never a Button: it is a button already."#, [
            .init("trigger", .slot),
            .init("content", .slot),
        ]),
        defineComponent(name: "Divider", description: "A thin horizontal rule between blocks.", props: []) { _, _ in LayoutDivider() },
        defineComponent(name: "Spacer", description: "Fills the free space in a Row.", props: []) { _, _ in LayoutSpacer() },
    ]
}
