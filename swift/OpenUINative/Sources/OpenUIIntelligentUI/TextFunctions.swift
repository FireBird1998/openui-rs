// Text: formatting and string edits. Upstream's lib/functions/text.ts, declared
// in its order. Each returns null until its ticket gives it an implementation.
import OpenUINative

extension IntelligentUI {
    static let textFunctions: [DefinedFunction] = [
        function("Format", #"Number as text: "number" 12,345.6, "currency" $12,345.68 (currency is a code like "EUR"), "percent" 45 -> 45%, "compact" 1.2M ($1.2M with a currency)"#, [
            .init("value", .number),
            .init("kind", .words(["number", "currency", "percent", "compact"]), optional: true),
            .init("decimals", .number, optional: true),
            .init("currency", .string, optional: true),
        ], returns: .string),
        function("Join", #"Joins the items (or their field) into text with sep (default ", "), skipping nulls"#, [
            .init("list", .list),
            .init("sep", .string, optional: true),
            .init("field", .string, optional: true),
        ], returns: .string),
        function("Plural", #"Count and word: @Plural(1, "guest") is "1 guest", 3 gives "3 guests"; plural defaults to word + "s""#, [
            .init("n", .number),
            .init("word", .string),
            .init("plural", .string, optional: true),
        ], returns: .string),
        function("Pad", #"Pads the start of value to length with char (default "0"), e.g. 7 -> "07""#, [
            .init("value", .date),
            .init("length", .number),
            .init("char", .string, optional: true),
        ], returns: .string),
        function("Upper", "Text in upper case", [
            .init("text", .string),
        ], returns: .string),
        function("Lower", "Text in lower case", [
            .init("text", .string),
        ], returns: .string),
        function("Trim", "Text without spaces at the start and end", [
            .init("text", .string),
        ], returns: .string),
        function("Capitalize", #"Each word with a capital first letter, e.g. "new york" is "New York""#, [
            .init("text", .string),
        ], returns: .string),
        function("Split", #"List of the parts of text between sep (default ","), each trimmed, empty parts dropped, e.g. @Split("a, b,,c") is ["a", "b", "c"]"#, [
            .init("text", .string),
            .init("sep", .string, optional: true),
        ], returns: .array(.string)),
        function("Replace", "Text with every find replaced by replacement", [
            .init("text", .string),
            .init("find", .string),
            .init("replacement", .string),
        ], returns: .string),
        function("Duration", #"Seconds as text: "short" 1h 20m, 4m 05s, 45s (default); "clock" 1:20:00, 4:05"#, [
            .init("seconds", .number),
            .init("style", .words(["short", "clock"]), optional: true),
        ], returns: .string),
        function("Default", #"value, or fallback when value is null, "" or not a number, e.g. an empty number input: @Default($weight, 0)"#, [
            .init("value", .any, optional: true),
            .init("fallback", .any),
        ], returns: .any),
    ]
}
