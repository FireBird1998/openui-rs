// Lists: copy, search and reshape the lists held in $variables. Upstream's lib/functions/lists.ts, declared
// in its order.
import Foundation
import OpenUINative

extension IntelligentUI {
    static func listFunctions(random: @escaping @Sendable () -> Double) -> [DefinedFunction] {
        [
            function("Append", "Copy of the list with item added at the end", [
                .init("list", .list),
                .init("item", .any),
            ], returns: .list) { args in
                try ListFunctions.join(args.listArgument(), [args.itemArgument("item")])
            },
            function("Prepend", "Copy of the list with item added at the start", [
                .init("list", .list),
                .init("item", .any),
            ], returns: .list) { args in
                try ListFunctions.join([args.itemArgument("item")], args.listArgument())
            },
            function("Concat", "Joins two lists", [
                .init("list", .list),
                .init("more", .list),
            ], returns: .list) { args in
                try ListFunctions.join(args.listArgument(), args.listArgument("more"))
            },
            function("Remove", #"Copy without the items whose field equals value; field "" compares the items themselves"#, [
                .init("list", .list),
                .init("field", .string),
                .init("value", .any),
            ], returns: .list) { args in
                let matches = try ListFunctions.matchField(args)
                return try .array(args.listArgument().filter { !matches($0) })
            },
            function("RemoveAt", "Copy without the item at index (0-based; negative counts from the end)", [
                .init("list", .list),
                .init("index", .number),
            ], returns: .list) { args in
                let list = try args.listArgument()
                let at = try args.numberArgument("index").rounded(.towardZero)
                let index = at < 0 ? Double(list.count) + at : at
                return .array(list.enumerated().compactMap { Double($0.offset) == index ? nil : $0.element })
            },
            function("Update", "Copy where items whose field equals value get setField set to setValue", [
                .init("list", .list),
                .init("field", .string),
                .init("value", .any),
                .init("setField", .string),
                .init("setValue", .any),
            ], returns: .list) { args in
                let matches = try ListFunctions.matchField(args)
                let field = try args.stringArgument("setField")
                let value = try args.itemArgument("setValue")
                return try .array(args.listArgument().map { item in
                    guard case var .object(entries) = item, matches(item) else { return item }
                    if let index = entries.firstIndex(where: { $0.key.utf8.elementsEqual(field.utf8) }) {
                        entries.replaceSubrange(index...index, with: [(field, value)])
                    } else {
                        entries.append((field, value))
                    }
                    return .object(entries)
                })
            },
            function("Toggle", "Adds item if missing, removes it if present (a set of strings or numbers)", [
                .init("list", .list),
                .init("item", .any),
            ], returns: .list) { args in
                let list = try args.listArgument()
                let item = try args.itemArgument("item")
                let key = ListFunctions.key(item)
                let kept = list.filter { ListFunctions.key($0) != key }
                return .array(kept.count == list.count ? list + [item] : kept)
            },
            function("Includes", "True if the list has item, or the text contains it", [
                .init("list", .union([.list, .string])),
                .init("item", .any),
            ], returns: .boolean) { args in
                let item = try args.itemArgument("item")
                if let text = args["list"]?.string {
                    let needle = try ListFunctions.text(item)
                    return .bool(needle.isEmpty || text.range(of: needle, options: .literal) != nil)
                }
                guard let list = args["list"]?.array else {
                    let method = ListFunctions.methodDescription(args["list"]?["some"])
                    throw FunctionError("list.some is not a function. (In 'list.some((x) => same(x, item))', 'list.some' is \(method))")
                }
                let key = ListFunctions.key(item)
                return .bool(list.contains { ListFunctions.key($0) == key })
            },
            function("Find", "First item whose field equals value, or null", [
                .init("list", .list),
                .init("field", .string),
                .init("value", .any),
            ], returns: .any) { args in
                let matches = try ListFunctions.matchField(args)
                return try args.listArgument().first(where: matches) ?? .null
            },
            function("IndexOf", "Index of item in the list, or -1", [
                .init("list", .list),
                .init("item", .any),
            ], returns: .number) { args in
                let key = try ListFunctions.key(args.itemArgument("item"))
                let index = try args.listArgument().firstIndex { ListFunctions.key($0) == key }
                return .number(Double(index ?? -1))
            },
            function("Take", "The first n items; a negative n takes the last n", [
                .init("list", .list),
                .init("n", .number),
            ], returns: .list) { args in
                let list = try args.listArgument()
                let n = try ListFunctions.count(args.numberArgument("n"), length: list.count)
                return .array(n >= 0 ? Array(list.prefix(n)) : Array(list.suffix(-n)))
            },
            function("Slice", "Items, or characters of text, from start up to end (not included); negative counts from the end", [
                .init("list", .union([.list, .string])),
                .init("start", .number),
                .init("end", .number, optional: true),
            ], returns: .union([.list, .string])) { args in
                try ListFunctions.slice(args)
            },
            function("Unique", "Distinct items in first-seen order", [
                .init("list", .list),
            ], returns: .list) { args in
                var seen = Set<[UInt8]>()
                return try .array(args.listArgument().filter { item in
                    // Unique uses String(item) for all top-level primitives.
                    let value = ListFunctions.primitiveText(item).map(JSONValue.string) ?? item
                    return seen.insert(ListFunctions.key(value)).inserted
                })
            },
            function("Reverse", "Copy in reverse order", [
                .init("list", .list),
            ], returns: .list) { args in
                try .array(args.listArgument().reversed())
            },
            function("Range", "Numbers from start up to end (not included) by step (default 1), e.g. @Range(0, 3) is [0, 1, 2]", [
                .init("start", .number),
                .init("end", .number),
                .init("step", .number, optional: true),
            ], returns: .numbers) { args in
                try ListFunctions.range(args)
            },
            function("Pick", "A random item, or null for an empty list; draws anew each time, so call it inside @Set", [
                .init("list", .list),
            ], returns: .any) { args in
                let list = try args.listArgument()
                guard !list.isEmpty else { return .null }
                let index = try ListFunctions.drawIndex(random, count: list.count)
                return list.dropFirst(index).first ?? .null
            },
            function("Shuffle", "Copy in random order; call it inside @Set", [
                .init("list", .list),
            ], returns: .list) { args in
                var list = try args.listArgument()
                for size in stride(from: list.count, to: 1, by: -1) {
                    let index = try ListFunctions.drawIndex(random, count: size)
                    list.swapAt(size - 1, index)
                }
                return .array(list)
            },
            function("Flatten", "One list from a list of lists, e.g. [[1, 2], [3]] is [1, 2, 3]", [
                .init("list", .list),
            ], returns: .list) { args in
                var result: [JSONValue] = []
                for item in try args.listArgument() {
                    let items = item.array ?? [item]
                    try ListFunctions.checkLength(result.count, adding: items.count)
                    result.append(contentsOf: items)
                }
                return .array(result)
            },
            function("Chunk", "The list cut into lists of size items, e.g. for weeks of 7 days or pages", [
                .init("list", .list),
                .init("size", .number),
            ], returns: .array(.list)) { args in
                let list = try args.listArgument()
                let n = try args.numberArgument("size").rounded(.towardZero)
                guard n >= 1 else { throw FunctionError("size must be 1 or more") }
                var tail = list[...]
                var chunks: [JSONValue] = []
                let size = ListFunctions.count(n, length: list.count)
                while !tail.isEmpty {
                    chunks.append(.array(Array(tail.prefix(size))))
                    tail = tail.dropFirst(size)
                }
                return .array(chunks)
            },
        ]
    }
}

private extension JSONValue {
    func listArgument(_ name: String = "list") throws -> [JSONValue] {
        guard let list = self[name]?.array else { throw FunctionError("\(name) is not a list") }
        return list
    }

    func itemArgument(_ name: String) throws -> JSONValue {
        guard let value = self[name], value != .null else { throw FunctionError("\(name) is missing") }
        return value
    }
}

enum ListFunctions {
    private struct FieldSegment {
        let spelling: String
        let index: Int?

        init(_ spelling: String, parseIndex: (String) -> Int?) {
            self.spelling = spelling
            if let i = parseIndex(spelling), i >= 0, String(i) == spelling {
                index = i
            } else {
                index = nil
            }
        }
    }

    static func matchField(_ args: JSONValue, parseIndex: (String) -> Int? = Int.init) throws -> (JSONValue) -> Bool {
        let field = try args.stringArgument("field")
        let target = try key(args.itemArgument("value"))
        if field.isEmpty { return { key($0) == target } }
        let path = field.utf8.split(separator: 46, omittingEmptySubsequences: false).map {
            FieldSegment(String(decoding: $0, as: UTF8.self), parseIndex: parseIndex)
        }
        return { item in
            guard let value = read(item, path: path) else { return false }
            return key(value) == target
        }
    }
}

private extension ListFunctions {
    static let maxItems = 10_000

    static func drawIndex(_ random: @Sendable () -> Double, count: Int) throws -> Int {
        let draw = random()
        guard (0..<1).contains(draw),
              let index = Int(exactly: (draw * Double(count)).rounded(.down)), index < count
        else { throw FunctionError("random draw must be in [0, 1)") }
        return index
    }

    static func checkLength(_ count: Int, adding: Int) throws {
        guard count <= maxItems - adding else { throw FunctionError("more than 10000 items") }
    }

    static func join(_ first: [JSONValue], _ second: [JSONValue]) throws -> JSONValue {
        try checkLength(first.count, adding: second.count)
        return .array(first + second)
    }

    // The host JSON byte limit bounds collection sizes. Clamp before converting
    // an arbitrary JavaScript number to a Swift index.
    static func count(_ number: Double, length: Int) -> Int {
        Int(max(-Double(length), min(number.rounded(.towardZero), Double(length))))
    }

    static func slice(_ args: JSONValue) throws -> JSONValue {
        let start = try args.numberArgument("start")
        let end = args["end"] == nil ? nil : try args.numberArgument("end")
        func bounds(_ length: Int) -> Range<Int> {
            func index(_ n: Double) -> Int {
                let offset = count(n, length: length)
                return offset < 0 ? length + offset : offset
            }
            let lower = index(start)
            return lower..<max(lower, end.map(index) ?? length)
        }
        if let list = args["list"]?.array {
            return .array(Array(list[bounds(list.count)]))
        }
        if let units = args["list"]?.string?.utf16 {
            let range = bounds(units.count)
            let first = units.index(units.startIndex, offsetBy: range.lowerBound)
            let last = units.index(first, offsetBy: range.count)
            // JavaScript slices code units. Decoding replaces each unpaired half with U+FFFD.
            return .string(String(decoding: units[first..<last], as: UTF16.self))
        }
        let method = methodDescription(args["list"]?["slice"])
        throw FunctionError("list.slice is not a function. (In 'list.slice(Math.trunc(start), to)', 'list.slice' is \(method))")
    }

    static func range(_ args: JSONValue) throws -> JSONValue {
        let start = try args.numberArgument("start")
        let end = try args.numberArgument("end")
        let step = args["step"] == nil ? 1 : try args.numberArgument("step")
        guard step.isFinite, step != 0 else { throw FunctionError("step must be a non-zero number") }
        let count = max(0, ((end - start) / step).rounded(.up))
        guard count <= Double(maxItems) else { throw FunctionError("more than 10000 items") }
        let whole = start.rounded() == start && step.rounded() == step
        return .array((0..<Int(count)).map { i in
            let value = start + Double(i) * step
            return .number(whole ? value : precision12(value))
        })
    }

    // Number(x.toPrecision(12)). A tie goes to the larger magnitude. Read
    // the exact decimal expansion (a binary64 needs at most 1074 fractional
    // digits), then round once before converting the decimal back to binary64.
    static func precision12(_ value: Double) -> Double {
        guard value != 0 else { return 0 }
        let parts = String(format: "%.1074e", abs(value)).split(separator: "e")
        guard let significand = parts.first, let exponent = parts.last.flatMap({ Int($0) }) else { return value }
        let digits = significand.utf8.filter { $0 != UInt8(ascii: ".") }
        var mantissa = digits.prefix(12).reduce(Int64(0)) { $0 * 10 + Int64($1 - UInt8(ascii: "0")) }
        if let next = digits.dropFirst(12).first, next >= UInt8(ascii: "5") { mantissa += 1 }
        let rounded = Double("\(mantissa)e\(exponent - 11)") ?? abs(value)
        return value.sign == .minus ? -rounded : rounded
    }
}

private extension ListFunctions {
    // Swift and JavaScript both print the shortest round-tripping digits of
    // a Double. JavaScript uses ordinary decimal notation for exponents
    // -6 through 20, and no leading zero in an exponent.
    static func numberText(_ n: Double) -> String {
        if n.isNaN { return "NaN" }
        if !n.isFinite { return n < 0 ? "-Infinity" : "Infinity" }
        if n == 0 { return "0" }
        let parts = String(abs(n)).split(separator: "e")
        guard let significand = parts.first else { return "" }
        let point = significand.firstIndex(of: ".") ?? significand.endIndex
        var decimal = significand.distance(from: significand.startIndex, to: point)
        if let exp = parts.dropFirst().first.flatMap({ Int($0) }) { decimal += exp }
        var digits = significand.filter { $0 != "." }[...]
        while digits.first == "0" {
            digits = digits.dropFirst()
            decimal -= 1
        }
        while digits.last == "0" { digits = digits.dropLast() }
        let sign = n < 0 ? "-" : ""
        if decimal <= -6 || decimal > 21 {
            let exponent = decimal - 1
            let tail = digits.dropFirst()
            return sign + digits.prefix(1) + (tail.isEmpty ? "" : "." + tail)
                + "e" + (exponent >= 0 ? "+" : "") + String(exponent)
        }
        if decimal <= 0 { return sign + "0." + String(repeating: "0", count: -decimal) + digits }
        if decimal >= digits.count { return sign + digits + String(repeating: "0", count: decimal - digits.count) }
        return sign + digits.prefix(decimal) + "." + digits.dropFirst(decimal)
    }

    static func primitiveText(_ item: JSONValue) -> String? {
        switch item {
        case .null: "null"
        case .bool(let b): b ? "true" : "false"
        case .number(let n): numberText(n)
        case .string(let s): s
        default: nil
        }
    }

    // Bun's TypeError describes the property that was called.
    static func methodDescription(_ value: JSONValue?) -> String {
        switch value {
        case nil: "undefined"
        case .array: "an instance of Array"
        case .object: "an instance of Object"
        case .string(let s): "\"\(s)\""
        case .some(let item): primitiveText(item) ?? "undefined"
        }
    }

    /// String(item) for a text Includes. Array.join renders a null slot as
    /// empty and joins nested lists on their own commas.
    static func text(_ item: JSONValue) throws -> String {
        var out = ""
        var work = [ArraySlice([item])]
        while let list = work.popLast(), let value = list.first {
            if list.startIndex > 0 { out += "," }
            if list.count > 1 { work.append(list.dropFirst()) }
            switch value {
            case .array(let array):
                if !array.isEmpty { work.append(array[...]) }
            case .object(let entries):
                guard !entries.contains(where: { $0.key == "toString" }) else {
                    throw FunctionError("No default value")
                }
                out += "[object Object]"
            case .null: break
            default: out += primitiveText(value) ?? ""
            }
        }
        return out
    }

    // A prefix-free comparison key for upstream's same(). Strings keep their
    // exact bytes. Numbers share the key of their JS text. Object order does
    // not matter. Building each item's key once lets Unique avoid upstream's
    // quadratic scan over all previously kept objects.
    private enum KeyPart {
        case value(JSONValue), field(String)
    }

    static func key(_ value: JSONValue) -> [UInt8] {
        var out: [UInt8] = []
        var work: [KeyPart] = [.value(value)]
        func appendText(_ text: String) {
            out.append(contentsOf: String(text.utf8.count).utf8)
            out.append(UInt8(ascii: ":"))
            out.append(contentsOf: text.utf8)
        }
        while let part = work.popLast() {
            switch part {
            case .field(let name): appendText(name)
            case .value(.null): out.append(UInt8(ascii: "n"))
            case .value(.bool(let flag)): out.append(flag ? UInt8(ascii: "t") : UInt8(ascii: "f"))
            case .value(.number(let n)): appendText(numberText(n))
            case .value(.string(let s)): appendText(s)
            case .value(.array(let items)):
                out.append(UInt8(ascii: "["))
                appendText(String(items.count))
                work.append(contentsOf: items.reversed().map(KeyPart.value))
            case .value(.object(let entries)):
                out.append(UInt8(ascii: "{"))
                appendText(String(entries.count))
                for entry in entries.sorted(by: { $1.key.utf8.lexicographicallyPrecedes($0.key.utf8) }) {
                    work.append(.value(entry.value))
                    work.append(.field(entry.key))
                }
            }
        }
        return out
    }

    // read() stops at primitives. Arrays have their own length and index
    // properties. Inherited methods are functions and cannot equal a JSON
    // value. The two prototype objects themselves can equal [] or {}.
    private static func read(_ item: JSONValue, path: [FieldSegment]) -> JSONValue? {
        var value = item
        var prototype = false
        for segment in path {
            let name = segment.spelling
            switch value {
            case .array(let items):
                if name == "length" { value = .number(Double(items.count)) }
                else if name == "__proto__" { value = prototype ? .object([]) : .array([]); prototype = true; continue }
                else if let i = segment.index, let next = items.dropFirst(i).first {
                    value = next
                } else { return nil }
            case .object(let entries):
                if let entry = entries.first(where: { $0.key.utf8.elementsEqual(name.utf8) }) {
                    value = entry.value
                } else if name == "__proto__" {
                    value = prototype ? .null : .object([])
                    prototype = true
                    continue
                } else { return nil }
            default: return nil
            }
            prototype = false
        }
        return value
    }
}
