// Data: shaping Query rows for tables and charts. Upstream's lib/functions/data.ts, declared
// in its order.
import Foundation
import JavaScriptCore
import OpenUINative

extension IntelligentUI {
    static func dataFunctions(timeZone: TimeZone) -> [DefinedFunction] {
        [
            function("GroupBy", #"One row per distinct key, in first-seen order; value is agg of field (default "sum"), or the row count without a field"#, [
                .init("rows", .list),
                .init("key", .string),
                .init("field", .string, optional: true),
                .init("agg", .agg, optional: true),
            ], returns: .array(.object([.init("key", .date), .init("value", .number)])), implementation: DataFunctions.groupBy),
            function("Pivot", "One row per rowKey value, under key, with a column per colKey value holding agg of field, for stacked or grouped charts", [
                .init("rows", .list),
                .init("rowKey", .string),
                .init("colKey", .string),
                .init("field", .string, optional: true),
                .init("agg", .agg, optional: true),
            ], returns: .array(.record(.date)), implementation: DataFunctions.pivot),
            function("Top", #"The n rows with the largest field ("asc": the smallest)"#, [
                .init("rows", .list),
                .init("field", .string),
                .init("n", .number),
                .init("direction", .words(["desc", "asc"]), optional: true),
            ], returns: .list, implementation: DataFunctions.top),
            function("Between", #"Rows whose field is from min to max, both included; compares numbers, dates by day ("YYYY-MM-DD", ISO or a timestamp), or text. Leave min or max null for an open end"#, [
                .init("rows", .list),
                .init("field", .string),
                .init("min", .any, optional: true),
                .init("max", .any, optional: true),
            ], returns: .list) { try DataFunctions.between($0, zone: timeZone) },
            function("Bucket", #"Rows per day, week (from Monday) or month of dateField, in date order, empty periods 0 for sum and count; key is "YYYY-MM-DD", or "YYYY-MM" for months"#, [
                .init("rows", .list),
                .init("dateField", .string),
                .init("unit", .words(["day", "week", "month"])),
                .init("field", .string, optional: true),
                .init("agg", .agg, optional: true),
            ], returns: .array(.object([.init("key", .date), .init("value", .number)]))) { try DataFunctions.bucket($0, zone: timeZone) },
            function("Cumulative", #"Copy of the rows with field replaced by its running total (field "" for a list of numbers)"#, [
                .init("rows", .list),
                .init("field", .string),
            ], returns: .list, implementation: DataFunctions.cumulative),
            function("MovingAvg", #"Copy of the rows with field replaced by the average of its last window values (field "" for a list of numbers)"#, [
                .init("rows", .list),
                .init("field", .string),
                .init("window", .number),
            ], returns: .list, implementation: DataFunctions.movingAvg),
            function("Change", "Percent change from previous to current, e.g. @Change(120, 100) is 20; null when previous is 0", [
                .init("current", .number),
                .init("previous", .number),
            ], returns: .nullable(.number), implementation: DataFunctions.change),
            function("Histogram", "Counts numbers into equal-width bins as {key, value} rows: key is the bin's start, value the count; min and max fix the range, default the data's", [
                .init("values", .numbers),
                .init("bins", .number),
                .init("min", .number, optional: true),
                .init("max", .number, optional: true),
            ], returns: .array(.object([.init("key", .string), .init("value", .number)])), implementation: DataFunctions.histogram),
            function("Stats", "Summary of numbers, all zeros for an empty list", [
                .init("values", .numbers),
            ], returns: .object([.init("count", .number), .init("min", .number), .init("max", .number), .init("mean", .number), .init("median", .number), .init("stdev", .number)]), implementation: DataFunctions.stats),
        ]
    }
}

enum DataFunctions {
    static func stats(_ args: JSONValue) throws -> JSONValue {
        let values = try args.listArgument("values").compactMap(\.number).filter(\.isFinite)
        let count = Double(values.count)
        let sorted = values.sorted()
        let middle = sorted.count / 2
        let upper = sorted.dropFirst(middle).first ?? 0
        let median = sorted.count.isMultiple(of: 2) ? ((sorted.prefix(middle).last ?? 0) + upper) / 2 : upper
        let mean = values.isEmpty ? 0 : values.reduce(0, +) / count
        let variance = values.reduce(0.0) { sum, value in
            let delta = value - mean
            return sum + delta * delta
        }
        return .object([
            ("count", .number(count)), ("min", .number(sorted.first ?? 0)), ("max", .number(sorted.last ?? 0)),
            ("mean", NumberFunctions.jsonNumber(mean)), ("median", NumberFunctions.jsonNumber(median)),
            ("stdev", NumberFunctions.jsonNumber(values.isEmpty ? 0 : (variance / count).squareRoot())),
        ])
    }
}

private extension DataFunctions {
    static func reader(_ field: String) -> (JSONValue) -> JSONValue? {
        ListFunctions.fieldReader(field, method: ListFunctions.methodText)
    }

    // helpers.toNum accepts only numbers and non-blank numeric strings. Swift
    // also accepts signed hexadecimal and hex floats, which Number() rejects.
    static func numeric(_ value: JSONValue?) -> Double? {
        if let n = value?.number { return n.isFinite ? n : nil }
        guard let text = value?.string else { return nil }
        let trimmed = String(decoding: JavaScript.trim(text), as: UTF16.self)
        let bytes = trimmed.utf8
        guard let first = bytes.first else { return nil }
        if first == 48, let prefix = bytes.dropFirst().first {
            let bits: Int
            switch prefix {
            case 88, 120: bits = 4
            case 79, 111: bits = 3
            case 66, 98: bits = 1
            default: bits = 0
            }
            if bits > 0 {
                let digits = bytes.dropFirst(2)
                guard !digits.isEmpty else { return nil }
                // Retain the leading significand, guard bit and sticky bit for
                // correctly rounded strings longer than an integer register.
                var leading: UInt64 = 0
                var length = 0
                var sticky = false
                for byte in digits {
                    let digit: UInt8
                    switch byte {
                    case 48...57: digit = byte - 48
                    case 65...70: digit = byte - 55
                    case 97...102: digit = byte - 87
                    default: return nil
                    }
                    guard digit < 1 << bits else { return nil }
                    for shift in (0..<bits).reversed() {
                        let bit = UInt64((digit >> shift) & 1)
                        if length == 0 && bit == 0 { continue }
                        length += 1
                        if length <= 54 { leading = leading * 2 + bit }
                        else if bit != 0 { sticky = true }
                    }
                }
                if length <= 53 { return Double(leading) }
                let roundUp = leading & 1 != 0 && (sticky || leading & 2 != 0)
                let n = Double(sign: .plus, exponent: length - 53, significand: Double((leading >> 1) + (roundUp ? 1 : 0)))
                return n.isFinite ? n : nil
            }
        }
        guard bytes.allSatisfy({ (48...57).contains($0) || [43, 45, 46, 69, 101].contains($0) }),
              let n = Double(trimmed), n.isFinite else { return nil }
        return n
    }

    enum Key: Hashable {
        case number(Double), text(Data)

        init(_ raw: JSONValue?) throws {
            if let n = raw?.number {
                self = .number(n)
                return
            }
            let s: String
            if let raw, raw != .null, raw.string != "" { s = try ListFunctions.text(raw) }
            else { s = "Unknown" }
            self = .text(Data(s.utf8))
        }

        var value: JSONValue {
            switch self {
            case .number(let n): .number(n)
            case .text(let bytes): .string(String(decoding: bytes, as: UTF8.self))
            }
        }

        var spelling: String {
            switch self {
            case .number(let n): JavaScript.numberString(n)
            case .text(let bytes): String(decoding: bytes, as: UTF8.self)
            }
        }
    }

    static func groups(_ rows: [JSONValue], by read: (JSONValue) -> JSONValue?) throws -> [(Key, [JSONValue])] {
        var order: [Key] = []
        var members: [Key: [JSONValue]] = [:]
        for row in rows {
            let k = try Key(read(row))
            if members[k] == nil { order.append(k) }
            members[k, default: []].append(row)
        }
        return order.map { ($0, members[$0] ?? []) }
    }

    struct Aggregate {
        let mode: String
        let read: (JSONValue) -> JSONValue?

        init(_ args: JSONValue) {
            let field = args["field"]?.string ?? ""
            mode = args["agg"]?.string ?? (field.isEmpty ? "count" : "sum")
            read = reader(field)
        }

        func callAsFunction(_ rows: [JSONValue]) -> JSONValue {
            if mode == "count" { return .number(Double(rows.count)) }
            let values = rows.lazy.compactMap { numeric(read($0)) }
            switch mode {
            case "min": return .number(values.min() ?? 0)
            case "max": return .number(values.max() ?? 0)
            default:
                var count = 0
                var sum = 0.0
                for value in values {
                    count += 1
                    sum += value
                }
                return NumberFunctions.jsonNumber(mode == "avg" && count > 0 ? sum / Double(count) : sum)
            }
        }
    }

    static func groupBy(_ args: JSONValue) throws -> JSONValue {
        let aggregate = Aggregate(args)
        return try .array(groups(args.listArgument("rows"), by: reader(args.stringArgument("key"))).map { key, rows in
            .object([("key", key.value), ("value", aggregate(rows))])
        })
    }

    static func arrayIndex(_ name: String) -> UInt32? {
        guard let n = UInt32(name), n < UInt32.max, String(n) == name else { return nil }
        return n
    }

    static func pivot(_ args: JSONValue) throws -> JSONValue {
        let rows = try args.listArgument("rows")
        let column = try reader(args.stringArgument("colKey"))
        let aggregate = Aggregate(args)
        // Object assignment stringifies keys. Numeric and string Map keys may
        // overwrite the same column; __proto__ ignores a numeric assignment.
        var names = ["key"]
        var seen: Set<Data> = [Data("key".utf8)]
        var hasKeyColumn = false
        for (key, _) in try groups(rows, by: column) {
            let name = key.spelling
            if name == "key" { hasKeyColumn = true }
            if name != "__proto__", seen.insert(Data(name.utf8)).inserted { names.append(name) }
        }
        var columns = names.map { (name: $0, bytes: Data($0.utf8), index: arrayIndex($0)) }
        let grouped = try groups(rows, by: reader(args.stringArgument("rowKey")))
        let bytesPerRow = columns.reduce(0) { $0 + $1.bytes.count + 4 }
        guard grouped.count <= defaultHostByteLimit / bytesPerRow else {
            throw OpenUIError(.limitExceeded, "host function result byte limit")
        }
        if columns.contains(where: { $0.index != nil }) {
            columns.sort { ($0.index ?? .max) < ($1.index ?? .max) }
        }
        return try .array(grouped.map { key, members in
            var cells: [Data: JSONValue] = [:]
            for (k, cell) in try groups(members, by: column) { cells[Data(k.spelling.utf8)] = aggregate(cell) }
            if !hasKeyColumn { cells[Data("key".utf8)] = key.value }
            return .object(columns.map { ($0.name, cells[$0.bytes] ?? .number(0)) })
        })
    }

    static func top(_ args: JSONValue) throws -> JSONValue {
        let read = try reader(args.stringArgument("field"))
        let rows = try args.listArgument("rows")
        let n = try args.numberArgument("n")
        let count = Int(max(0, min(Double(rows.count), n)))
        guard count > 0 else { return .array([]) }
        let ascending = args["direction"]?.string == "asc"
        // Swift's stable sort keeps ties (including missing values) in input order.
        let ranked = rows.map { (row: $0, n: numeric(read($0))) }.sorted { a, b in
            guard let x = a.n else { return false }
            guard let y = b.n else { return true }
            return ascending ? x < y : x > y
        }
        return .array(ranked.prefix(count).map(\.row))
    }

    static func change(_ args: JSONValue) throws -> JSONValue {
        let current = try args.numberArgument("current")
        let previous = try args.numberArgument("previous")
        return previous == 0 ? .null : try NumberFunctions.finite((current - previous) / abs(previous) * 100)
    }
}

extension DataFunctions {
    // The maintainer approved this limit on 2026-11-24. Exact agreement with
    // upstream for inexact sums requires the ordered fold, with worst-case
    // work proportional to rows times window width.
    static let orderedFoldWorkLimit = (perRow: 1_000, perCall: 64_000_000)

    // The inputs have already gone through helpers.toNum (finite, or zero).
    // Floating-point subtraction can only undo an addition if no bits were
    // lost. Bound every partial sum by window * maxMagnitude in units of the
    // smallest set bit in the input. If it fits in 53 bits, every window's
    // left-to-right reduction and both sliding updates are exact.
    private static func sumsAreExact(_ values: [Double], window: Int) -> Bool {
        var highest = Int.min, lowest = Int.max
        for value in values where value != 0 {
            highest = max(highest, value.exponent)
            lowest = min(lowest, value.exponent - value.significandWidth)
        }
        guard highest != .min else { return true }
        let growth = Int.bitWidth - (window - 1).leadingZeroBitCount
        let sumExponent = highest + growth
        return sumExponent <= Double.greatestFiniteMagnitude.exponent && sumExponent - lowest <= Double.significandBitCount
    }

    static func movingAverages(_ values: [Double], window: Int,
                               add: (Double, Double) -> Double = (+)) throws -> [Double] {
        guard !values.isEmpty, (1...values.count).contains(window) else { return [] }
        let slides = values.count - window
        let slideExactly = slides > 0 && (window == 1 || sumsAreExact(values, window: window))
        if slides > 0 && !slideExactly {
            // Bound the row count before multiplying and use division to
            // admit the planned cost (slides + 1) * window without overflow.
            let limit = orderedFoldWorkLimit
            let allowance = min(values.count, limit.perCall / limit.perRow) * limit.perRow
            guard window <= allowance / (slides + 1) else {
                throw OpenUIError(.limitExceeded, "host function work limit")
            }
        }

        var sum = 0.0
        var averages: [Double] = []
        averages.reserveCapacity(values.count)
        for (index, value) in values.prefix(window).enumerated() {
            sum = add(sum, value)
            averages.append(sum / Double(index + 1))
        }
        for (start, pair) in zip(values, values.dropFirst(window)).enumerated() {
            if slideExactly {
                let (outgoing, incoming) = pair
                sum = add(add(sum, -outgoing), incoming)
            } else {
                sum = values.dropFirst(start + 1).prefix(window).reduce(0, add)
            }
            averages.append(sum / Double(window))
        }
        return averages
    }
}

private extension DataFunctions {
    // {...row, [field]: value} spreads the indexed UTF-16 units of a string
    // and the elements of an array; scalar numbers, booleans and null have no keys.
    static func mapField(_ rows: [JSONValue], _ field: String, _ values: [Double]) throws -> JSONValue {
        if field.isEmpty { return .array(values.map(NumberFunctions.jsonNumber)) }
        guard rows.count <= defaultHostByteLimit / (field.utf8.count + 4) else {
            throw OpenUIError(.limitExceeded, "host function result byte limit")
        }
        let fieldIndex = arrayIndex(field)
        var output: [JSONValue] = []
        var bytesLeft = defaultHostByteLimit
        for (row, n) in zip(rows, values) {
            var entries: [(key: String, value: JSONValue)]
            switch row {
            case .object(let fields): entries = fields
            case .array(let items): entries = items.enumerated().map { (String($0.offset), $0.element) }
            case .string(let text):
                guard text.utf16.count <= bytesLeft / 5 else { throw OpenUIError(.limitExceeded, "host function result byte limit") }
                entries = text.utf16.enumerated().map { (String($0.offset), .string(String(decoding: [$0.element], as: UTF16.self))) }
            default: entries = []
            }
            let entry = (key: field, value: NumberFunctions.jsonNumber(n))
            if let i = entries.firstIndex(where: { $0.key.utf8.elementsEqual(field.utf8) }) {
                entries.replaceSubrange(i...i, with: [entry])
            } else if let fieldIndex, let i = entries.firstIndex(where: { (arrayIndex($0.key) ?? .max) > fieldIndex }) {
                entries.insert(entry, at: i)
            } else {
                entries.append(entry)
            }
            let minimumBytes = entries.reduce(0) { $0 + $1.key.utf8.count + 4 }
            guard minimumBytes <= bytesLeft else { throw OpenUIError(.limitExceeded, "host function result byte limit") }
            bytesLeft -= minimumBytes
            output.append(.object(entries))
        }
        return .array(output)
    }

    static func cumulative(_ args: JSONValue) throws -> JSONValue {
        let rows = try args.listArgument("rows")
        let field = try args.stringArgument("field")
        let read = reader(field)
        var total = 0.0
        let totals = rows.map { row in
            total += numeric(read(row)) ?? 0
            return total
        }
        return try mapField(rows, field, totals)
    }

    static func movingAvg(_ args: JSONValue) throws -> JSONValue {
        let w = try args.numberArgument("window").rounded(.towardZero)
        guard w >= 1 else { throw FunctionError("window must be at least 1") }
        let rows = try args.listArgument("rows")
        let field = try args.stringArgument("field")
        let read = reader(field)
        let values = rows.map { numeric(read($0)) ?? 0 }
        let averages = try movingAverages(values, window: Int(min(Double(values.count), w)))
        return try mapField(rows, field, averages)
    }

    // Number(x.toFixed(places)) rounds the exact binary value, breaking a
    // decimal tie away from zero. Binary64's full fractional expansion fits
    // in 1074 decimal digits, including every subnormal.
    static func fixed(_ n: Double, places: Int) -> Double {
        guard n.isFinite, n != 0, abs(n) < 1e21 else { return n }
        let exact = String(format: "%.1074f", abs(n)).utf8
        let whole = exact.prefix { $0 != 46 }
        let fraction = exact.dropFirst(whole.count + 1)
        var digits = Array(whole) + Array(fraction.prefix(places))
        if let next = fraction.dropFirst(places).first, next >= 53 {
            var carry = true
            digits = digits.reversed().map { digit in
                guard carry else { return digit }
                carry = digit == 57
                return carry ? 48 : digit + 1
            }.reversed()
            if carry { digits.insert(49, at: 0) }
        }
        let rounded = Double(String(decoding: digits, as: UTF8.self) + "e-\(places)") ?? abs(n)
        return n < 0 ? -rounded : rounded
    }

    static func histogram(_ args: JSONValue) throws -> JSONValue {
        let bins = try args.numberArgument("bins").rounded(.towardZero)
        guard (1...500).contains(bins) else { throw FunctionError("bins must be from 1 to 500") }
        let values = try args.listArgument("values").compactMap(\.number).filter(\.isFinite)
        let lo = args["min"]?.number ?? values.min() ?? 0
        let maxValue = args["max"]?.number ?? values.max() ?? 1
        let hi = maxValue <= lo ? lo + 1 : maxValue
        let width = (hi - lo) / bins
        let decimals = max(0, 1 - floor(log10(width)))
        guard decimals <= 100 else { throw FunctionError("toFixed() argument must be between 0 and 100") }
        var counts = [Int: Int]()
        for value in values where value >= lo && value <= hi {
            let index = floor((value - lo) / width)
            if !index.isNaN { counts[Int(min(bins - 1, index)), default: 0] += 1 }
        }
        return .array((0..<Int(bins)).map { i in
            let key = fixed(lo + Double(i) * width, places: Int(decimals))
            return .object([("key", .string(JavaScript.numberString(key))), ("value", .number(Double(counts[i] ?? 0)))])
        })
    }
}

// A failed parse skips a row. A valid instant whose local midnight was
// clipped by Date.UTC participates in the comparison as NaN.
private extension DateFunctions {
    func dataDay(_ value: JSONValue) -> Double? {
        guard value.number != nil || value.string != nil, let parsed = try? parse(value) else { return nil }
        return parsed.day.map(Double.init) ?? .nan
    }
}

private enum DataCalendar {
    static func bounds(_ starts: [Int?]) -> (Int, Int)? {
        guard starts.contains(nil) else {
            let days = starts.compactMap { $0 }
            guard let first = days.min(), let last = days.max() else { return nil }
            return (first, last)
        }
        // NaN makes (a - b) an inconsistent comparator. Swift.sort requires
        // a strict weak ordering. For this time-clip edge ask JavaScriptCore's
        // Array.sort for the same two endpoints data.ts uses.
        let compare: @convention(block) (Double, Double) -> Double = { $0 - $1 }
        guard let context = JSContext(),
              let array = JSValue(object: starts.map { $0.map(Double.init) ?? .nan }, in: context),
              let sorted = array.invokeMethod("sort", withArguments: [compare]),
              let first = sorted.atIndex(0).flatMap({ Int(exactly: $0.toDouble()) }),
              let last = sorted.atIndex(starts.count - 1).flatMap({ Int(exactly: $0.toDouble()) }) else { return nil }
        return (first, last)
    }

    static func start(_ day: Int, unit: String) -> Int {
        switch unit {
        case "week": return day - floorMod(day + 3, 7)
        case "month":
            let date = CivilDate(day: day)
            return CivilDate.days(year: date.utcYear, month: date.month, date: 1)
        default: return day
        }
    }

    static func next(_ day: Int, unit: String) -> Int {
        guard unit == "month" else { return day + (unit == "week" ? 7 : 1) }
        let date = CivilDate(day: day)
        return CivilDate.days(year: date.utcYear + (date.month == 12 ? 1 : 0),
                              month: date.month % 12 + 1, date: 1)
    }
}

extension DataFunctions {
    enum ComparisonForm { case number, isoPrefix, day, text }

    private struct ComparisonValue {
        let value: JSONValue
        let dates: DateFunctions
        let converted: (ComparisonForm) -> Void

        lazy var number: Double? = {
            converted(.number)
            return numeric(value)
        }()
        lazy var isoPrefix: Bool = {
            converted(.isoPrefix)
            guard let text = value.string else { return false }
            // data.ts tests exactly ten leading ASCII code units after trim.
            let prefix = JavaScript.trim(text).prefix(10)
            return prefix.count == 10 && prefix.enumerated().allSatisfy { i, unit in
                (i == 4 || i == 7) ? unit == 45 : (48...57).contains(unit)
            }
        }()
        lazy var day: Double? = {
            converted(.day)
            return dates.dataDay(value)
        }()
        private var units: [UInt16]?

        init(_ value: JSONValue, dates: DateFunctions, converted: @escaping (ComparisonForm) -> Void) {
            self.value = value
            self.dates = dates
            self.converted = converted
        }

        mutating func text() throws -> [UInt16] {
            if let units { return units }
            converted(.text)
            let text = try Array(ListFunctions.text(value).utf16)
            units = text
            return text
        }

        mutating func compare(to bound: inout Self) throws -> Double {
            let x = number, y = bound.number
            if let x, let y { return x - y }
            if (isoPrefix || bound.isoPrefix), (isoPrefix || x != nil), (bound.isoPrefix || y != nil),
               let left = day, let right = bound.day { return left - right }
            let left = try text(), right = try bound.text()
            for (a, b) in zip(left, right) where a != b { return a < b ? -1 : 1 }
            return Double(left.count - right.count)
        }
    }

    static func between(_ args: JSONValue, zone: TimeZone,
                        boundConverted: @escaping (ComparisonForm) -> Void = { _ in }) throws -> JSONValue {
        let read = try reader(args.stringArgument("field"))
        let dates = DateFunctions(zone: zone)
        var low = ComparisonValue(args["min"] ?? .null, dates: dates, converted: boundConverted)
        var high = ComparisonValue(args["max"] ?? .null, dates: dates, converted: boundConverted)
        return try .array(args.listArgument("rows").filter { row in
            guard let value = read(row), value != .null else { return false }
            var item = ComparisonValue(value, dates: dates, converted: { _ in })
            if low.value != .null, try !(item.compare(to: &low) >= 0) { return false }
            if high.value != .null, try !(item.compare(to: &high) <= 0) { return false }
            return true
        })
    }
}

private extension DataFunctions {
    static func bucket(_ args: JSONValue, zone: TimeZone) throws -> JSONValue {
        let read = try reader(args.stringArgument("dateField"))
        let unit = try args.stringArgument("unit")
        let agg = Aggregate(args)
        let dates = DateFunctions(zone: zone)
        // A valid instant can have a local midnight outside Date.UTC's
        // range. Keep its NaN period distinct from a date that failed parsing.
        var buckets: [Int?: [JSONValue]] = [:]
        var starts: [Int?] = []
        for row in try args.listArgument("rows") {
            guard let raw = read(row), let parsed = dates.dataDay(raw) else { continue }
            let start = Int(exactly: parsed).flatMap { day -> Int? in
                let period = DataCalendar.start(day, unit: unit)
                return unit == "month" && !CivilDate.inRange(period) ? nil : period
            }
            if buckets[start] == nil { starts.append(start) }
            buckets[start, default: []].append(row)
        }
        var result: [JSONValue] = []
        func append(_ day: Int) throws {
            guard result.count < 10_000 else { throw FunctionError("more than 10000 periods") }
            let value = buckets[day].map { agg($0) } ?? .number(0)
            let label = try CivilDate.isoDay(Double(day))
            result.append(.object([("key", .string(unit == "month" ? String(label.prefix(7)) : label)), ("value", value)]))
        }
        guard let (first, last) = DataCalendar.bounds(starts), first <= last else { return .array([]) }
        guard CivilDate.inRange(first) else { throw FunctionError("Invalid Date") }
        if agg.mode == "count" || agg.mode == "sum" {
            var day = first
            while day <= last {
                try append(day)
                day = DataCalendar.next(day, unit: unit)
            }
        } else {
            let januaryZero = CivilDate.days(year: 0, month: 1, date: 1)
            let february1900 = CivilDate.days(year: 1900, month: 2, date: 1)
            var occupied: [Int] = []
            for case let day? in starts where day >= first && day <= last {
                // Date.UTC maps year zero to 1900 when stepping from January
                // to February. Sparse aggregation skips the same empty span.
                if unit == "month", first < januaryZero, day > januaryZero, day < february1900 { continue }
                guard occupied.count < 10_000 else { throw FunctionError("more than 10000 periods") }
                occupied.append(day)
            }
            for day in occupied.sorted() { try append(day) }
        }
        return .array(result)
    }
}
