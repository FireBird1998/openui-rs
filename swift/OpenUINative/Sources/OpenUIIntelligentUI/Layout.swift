// Layout: upstream's lib/{Col,Row,Box,Grid,Card,Carousel,Popover,Divider,Spacer}.
import OpenUINative

extension IntelligentUI {
    static let layout: [DefinedComponent] = [
        component("Col", "Vertical layout. Without gap it spaces blocks like an article.", [
            .init("children", .children),
            .init("gap", .number, optional: true),
            .init("align", .align, optional: true),
        ]),
        component("Row", "Horizontal layout; items are vertically centered.", [
            .init("children", .children),
            .init("gap", .number, optional: true),
            .init("align", .align, optional: true),
            .init("justify", .justify, optional: true),
            .init("wrap", .boolean, optional: true),
        ]),
        component("Box", "Flex container, a column by default. radius also clips the content.", [
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
        ]),
        component("Grid", "Equal columns, 1 to 12 (default 2). With minChildWidth (px), columns drop on narrow screens.", [
            .init("children", .children),
            .init("columns", .number, optional: true),
            .init("minChildWidth", .number, optional: true),
            .init("gap", .number, optional: true),
        ]),
        component("Card", "Panel with a border; soft is a filled panel for summaries.", [
            .init("children", .children),
            .init("variant", .words(["outline", "soft"]), optional: true),
            .init("gap", .number, optional: true),
            .init("padding", .number, optional: true),
        ]),
        component("Carousel", "Horizontal row that scrolls and snaps, each child an item itemWidth px wide (default 240): photos, cards, options, days.", [
            .init("children", .children),
            .init("itemWidth", .number, optional: true),
            .init("gap", .number, optional: true),
        ]),
        component("Popover", #"Shows content in a small panel when the trigger is clicked: a definition, a source, details behind an Icon("info", "sm"). The trigger is text or an Icon, never a Button: it is a button already."#, [
            .init("trigger", .slot),
            .init("content", .slot),
        ]),
        component("Divider", "A thin horizontal rule between blocks.", []),
        component("Spacer", "Fills the free space in a Row.", []),
    ]
}
