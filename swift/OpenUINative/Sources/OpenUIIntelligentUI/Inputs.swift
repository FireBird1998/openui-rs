// Inputs: upstream's lib/{Button,IconButton,Pressable,Input,TextArea,Select,
// SegmentedControl,Slider,Checkbox,RadioGroup,DatePicker,Timer,Form}. An
// input's value is a binding (upstream's `reactive(...)`).
import OpenUINative
import SwiftUI

extension IntelligentUI {
    static let inputs: [DefinedComponent] = [
        component("Button", "icon is a lucide name. Without an action it sends its label to the assistant.", [
            .init("label", .string),
            .init("action", .action(), optional: true),
            .init("variant", .words(["solid", "outline", "soft", "ghost"]), optional: true),
            .init("color", .words(["primary", "danger", "success"]), optional: true),
            .init("size", .words(["sm", "md", "lg"]), optional: true),
            .init("icon", .string, optional: true),
            .init("disabled", .boolean, optional: true),
            .init("block", .boolean, optional: true),
        ]),
        component("IconButton", "Round icon-only button; label is read aloud. Needs an action.", [
            .init("icon", .string),
            .init("action", .action(), optional: true),
            .init("label", .string, optional: true),
            .init("variant", .words(["outline", "ghost", "solid"]), optional: true),
        ]),
        component("Pressable", "Makes any content clickable, such as a tile or a swatch. Needs an action.", [
            .init("children", .children),
            .init("action", .action(), optional: true),
            .init("selected", .boolean, optional: true),
            .init("padding", .number, optional: true),
            .init("background", .color, optional: true),
            .init("radius", .radius, optional: true),
        ]),
        defineComponent(name: "Input", description: "Text or number field; a number field stores numbers, and null when emptied.", props: [
            .init("value", .union([.string, .number]), binding: true),
            .init("label", .string, optional: true),
            .init("type", .words(["text", "number"]), optional: true),
            .init("placeholder", .string, optional: true),
            .init("rules", .rules, optional: true),
        ]) { element, path in IntelligentTextField(input: TextInput(element, path: path)).id(element.programIdentity) },
        defineComponent(name: "TextArea", description: "Multi-line text field; rows is 2 to 20 (default 3).", props: [
            .init("value", .string, binding: true),
            .init("label", .string, optional: true),
            .init("placeholder", .string, optional: true),
            .init("rows", .number, optional: true),
            .init("rules", .rules, optional: true),
        ]) { element, path in IntelligentTextField(input: TextInput(element, path: path)).id(element.programIdentity) },
        component("Select", "Dropdown, for long option lists.", [
            .init("value", .string, binding: true),
            .init("options", .options),
            .init("label", .string, optional: true),
            .init("placeholder", .string, optional: true),
            .init("rules", .rules, optional: true),
        ]),
        component("SegmentedControl", "2 to 4 short choices in a row.", [
            .init("value", .string, binding: true),
            .init("options", .options),
            .init("block", .boolean, optional: true),
        ]),
        component("Slider", #"Shows label and value; unit like "%" or "$" formats the value."#, [
            .init("value", .number, binding: true),
            .init("min", .number),
            .init("max", .number),
            .init("step", .number, optional: true),
            .init("label", .string, optional: true),
            .init("unit", .string, optional: true),
            .init("marks", .array(.object([.init("value", .number), .init("label", .string)])), optional: true),
        ]),
        component("Checkbox", "lineThrough strikes the label when checked, for to-do lists. rules takes only required.", [
            .init("checked", .boolean, binding: true),
            .init("label", .string),
            .init("lineThrough", .boolean, optional: true),
            .init("rules", .rules, optional: true),
        ]),
        component("RadioGroup", "pills for short choices; cards for choices that need a description line.", [
            .init("value", .string, binding: true),
            .init("options", .choices),
            .init("label", .string, optional: true),
            .init("variant", .words(["list", "pills", "cards"]), optional: true),
            .init("rules", .rules, optional: true),
        ]),
        component("DatePicker", #"Dates, min and max are "YYYY-MM-DD". rules takes only required."#, [
            .init("value", .string, binding: true),
            .init("label", .string, optional: true),
            .init("min", .string, optional: true),
            .init("max", .string, optional: true),
            .init("rules", .rules, optional: true),
        ]),
        component("Timer", "Countdown with start, pause and reset. It counts the bound seconds down to 0, so other statements can react, e.g. $left == 0.", [
            .init("seconds", .number, binding: true),
            .init("label", .string, optional: true),
        ]),
        defineComponent(name: "Form", description: "Groups fields; their values go with any @ToAssistant inside, once every field's rules pass.", props: [
            .init("children", .children),
        ]) { element, path in
            IntelligentFormScope(element: element, path: path).id(element.programIdentity)
        }.withFormName(IntelligentFormScope.name),
    ]
}
