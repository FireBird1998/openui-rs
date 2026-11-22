// Lists: copy, search and reshape the lists held in $variables. Upstream's lib/functions/lists.ts, declared
// in its order. Each returns null until its ticket gives it an implementation.
import OpenUINative

extension IntelligentUI {
    static let listFunctions: [DefinedFunction] = [
        function("Append", "Copy of the list with item added at the end", [
            .init("list", .list),
            .init("item", .any),
        ], returns: .list),
        function("Prepend", "Copy of the list with item added at the start", [
            .init("list", .list),
            .init("item", .any),
        ], returns: .list),
        function("Concat", "Joins two lists", [
            .init("list", .list),
            .init("more", .list),
        ], returns: .list),
        function("Remove", #"Copy without the items whose field equals value; field "" compares the items themselves"#, [
            .init("list", .list),
            .init("field", .string),
            .init("value", .any),
        ], returns: .list),
        function("RemoveAt", "Copy without the item at index (0-based; negative counts from the end)", [
            .init("list", .list),
            .init("index", .number),
        ], returns: .list),
        function("Update", "Copy where items whose field equals value get setField set to setValue", [
            .init("list", .list),
            .init("field", .string),
            .init("value", .any),
            .init("setField", .string),
            .init("setValue", .any),
        ], returns: .list),
        function("Toggle", "Adds item if missing, removes it if present (a set of strings or numbers)", [
            .init("list", .list),
            .init("item", .any),
        ], returns: .list),
        function("Includes", "True if the list has item, or the text contains it", [
            .init("list", .union([.list, .string])),
            .init("item", .any),
        ], returns: .boolean),
        function("Find", "First item whose field equals value, or null", [
            .init("list", .list),
            .init("field", .string),
            .init("value", .any),
        ], returns: .any),
        function("IndexOf", "Index of item in the list, or -1", [
            .init("list", .list),
            .init("item", .any),
        ], returns: .number),
        function("Take", "The first n items; a negative n takes the last n", [
            .init("list", .list),
            .init("n", .number),
        ], returns: .list),
        function("Slice", "Items, or characters of text, from start up to end (not included); negative counts from the end", [
            .init("list", .union([.list, .string])),
            .init("start", .number),
            .init("end", .number, optional: true),
        ], returns: .union([.list, .string])),
        function("Unique", "Distinct items in first-seen order", [
            .init("list", .list),
        ], returns: .list),
        function("Reverse", "Copy in reverse order", [
            .init("list", .list),
        ], returns: .list),
        function("Range", "Numbers from start up to end (not included) by step (default 1), e.g. @Range(0, 3) is [0, 1, 2]", [
            .init("start", .number),
            .init("end", .number),
            .init("step", .number, optional: true),
        ], returns: .numbers),
        function("Pick", "A random item, or null for an empty list; draws anew each time, so call it inside @Set", [
            .init("list", .list),
        ], returns: .any),
        function("Shuffle", "Copy in random order; call it inside @Set", [
            .init("list", .list),
        ], returns: .list),
        function("Flatten", "One list from a list of lists, e.g. [[1, 2], [3]] is [1, 2, 3]", [
            .init("list", .list),
        ], returns: .list),
        function("Chunk", "The list cut into lists of size items, e.g. for weeks of 7 days or pages", [
            .init("list", .list),
            .init("size", .number),
        ], returns: .array(.list)),
    ]
}
