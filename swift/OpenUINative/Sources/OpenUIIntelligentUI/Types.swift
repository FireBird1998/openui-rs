// The prop types the components share, as upstream's lib/schemas.ts,
// lib/controls.ts and lib/Svg/helpers.ts declare them. A named type prints
// under its name in the prompt (`Node[]`, `Color`, `Length`, `Radius`,
// `Rules`); the others print their shape.
import OpenUINative

extension PropType {
    /// Any component, or a list of them (from @Each). Printed as `Node`.
    static let node = PropType.named("Node", .any)
    /// A children slot. Printed as `Node[]`.
    static let children = PropType.array(.node)
    /// A token or a CSS color. Printed as `Color`.
    static let color = PropType.named("Color", .string)
    /// A px number or a CSS length such as "50%". Printed as `Length`.
    static let length = PropType.named("Length", .union([.number, .string]))
    /// A radius word or px. Printed as `Radius`.
    static let radius = PropType.named("Radius", .union([.words(["sm", "md", "lg", "xl", "full"]), .number]))
    /// How a photo fills its box in Image.
    static let fit = PropType.words(["cover", "contain"])
    /// A field's validation inside a Form. Printed as `Rules`.
    static let rules = PropType.named("Rules", .object([
        .init("required", .boolean, optional: true),
        .init("email", .boolean, optional: true),
        .init("url", .boolean, optional: true),
        .init("numeric", .boolean, optional: true),
        .init("min", .number, optional: true),
        .init("max", .number, optional: true),
        .init("minLength", .number, optional: true),
        .init("maxLength", .number, optional: true),
        .init("pattern", .string, optional: true),
    ]))
    static let align = PropType.words(["start", "center", "end", "stretch"])
    static let justify = PropType.words(["start", "center", "end", "between"])
    /// A slot that takes a component, a list of them, or plain text.
    static let slot = PropType.union([.string, .children])
    /// Text's text: a string, or a run of strings and inline components.
    static let richText = PropType.union([.string, .children])
    /// A control's options.
    static let options = PropType.array(.union([
        .string, .object([.init("value", .string), .init("label", .string)]),
    ]))
    /// RadioGroup's options, which may carry a description line.
    static let choices = PropType.array(.union([
        .string,
        .object([.init("value", .string), .init("label", .string), .init("description", .string, optional: true)]),
    ]))
    /// What a G holds: the shapes, in prompt order.
    static let shapes = PropType.array(.union(shapeNames.map(PropType.component)))
    /// What an Svg holds: the shapes and G.
    static let drawables = PropType.array(.union((shapeNames + ["G"]).map(PropType.component)))
}

/// The shapes, in prompt order (upstream's `shapeComponents`).
let shapeNames = ["Circle", "Ellipse", "Rect", "Line", "Path", "Polyline", "Polygon", "SvgText"]

/// A stroked shape's paint props, after its own.
let strokePaint: [PropDefinition] = [
    .init("stroke", .color, optional: true),
    .init("strokeWidth", .number, optional: true),
    .init("opacity", .number, optional: true),
]

/// A fillable shape's paint props, after its own (upstream's `paint`).
let paint: [PropDefinition] = [.init("fill", .color, optional: true)] + strokePaint
