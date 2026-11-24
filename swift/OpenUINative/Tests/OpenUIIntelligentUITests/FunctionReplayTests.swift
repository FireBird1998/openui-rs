// Upstream's own function results, replayed through the Rust runtime with
// this library's Swift functions registered. reference/intelligent-ui-functions.ts
// records them (scripts/regen-golden.sh runs it) from upstream's function
// source vendored at the library pin, through the vendored parser and
// evaluator: conformance/golden/intelligent-ui-functions.json. Each pair
// records a program with state-backed or inline arguments, the probe's
// evaluated value, and upstream's parse and evaluation errors.
//
// Every recorded function must be registered: implemented, or listed in
// `pending` until its ticket lands. A pending function's pairs only check
// that its call gives null.
import Foundation
import enum OpenUIFFI.FunctionError
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import Testing

/// Exact inventories of boundary cases in the Bun recording. Count each pair once,
/// even when the same tagged value occurs in both its arguments and its result.
private let jsonBoundaryPairs = ["Format": 2, "Plural": 2, "Includes": 1, "MixColor": 1, "Reverse": 1, "Slice": 3,
                                "Join": 4, "Pad": 2, "Upper": 1, "Lower": 1, "Trim": 1, "Capitalize": 1,
                                "Split": 5, "Replace": 3, "Default": 2, "Allocate": 3, "Convert": 1, "Random": 4]
private let topLevelNumberPairs = ["Format": 15, "Plural": 12, "MixColor": 3, "Range": 3, "Join": 9, "Pad": 12, "Split": 6,
                                   "Replace": 3, "Duration": 9, "Default": 7, "Pow": 4, "Sqrt": 1, "Random": 7,
                                   "Clamp": 4, "Percent": 4, "Lerp": 4, "Allocate": 5, "Payment": 4, "Convert": 4]

private let expectedReplayCounts = [
    "Format": 467, "Plural": 40,
    "Append": 9, "Prepend": 8, "Concat": 9, "Remove": 21, "RemoveAt": 13, "Update": 25,
    "Toggle": 21, "Includes": 37, "Find": 28, "IndexOf": 20, "Take": 13, "Slice": 29,
    "Unique": 21, "Reverse": 9, "Range": 25, "Pick": 8, "Shuffle": 9, "Flatten": 9,
    "Chunk": 16, "MixColor": 68,
    "Join": 37, "Pad": 35, "Upper": 16, "Lower": 22, "Trim": 33, "Capitalize": 38,
    "Split": 32, "Replace": 21, "Duration": 46, "Default": 25,
    "Pow": 23, "Sqrt": 11, "Random": 27, "Clamp": 13, "Percent": 14, "Lerp": 14,
    "Allocate": 39, "Payment": 20, "Convert": 92,
]

/// Individual pairs still waiting for their functions. A rule can already
/// have a passing example through another function.
private let pendingRulePairs: [String: Int] = [
    "pruned array items": 1,
]

/// Declared, returning null until their tickets land.
private let pending: Set<String> = [
    // dates.ts
    "Now", "Today", "FormatDate", "AddDays", "AddMonths", "DaysBetween", "MonthsBetween", "StartOf",
    // data.ts
    "GroupBy", "Pivot", "Top", "Between", "Bucket", "Cumulative", "MovingAvg", "Change", "Histogram", "Stats",
]

private let recording: JSONValue = {
    var directory = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { directory.deleteLastPathComponent() }
    let url = directory.appendingPathComponent("conformance/golden/intelligent-ui-functions.json")
    return (try? JSONValue.parse(String(contentsOf: url, encoding: .utf8))) ?? .null
}()

/// The component every recorded call is the value of.
private let probe = defineComponent(name: "Probe", description: "", props: [.init("value", .any)])

/// One recorded pair.
private struct Pair {
    let function: String
    let args: JSONValue
    let source: String
    let value: JSONValue
    let errors: [JSONValue]
    let parseErrors: [JSONValue]
    let zone: String?
    let draws: [JSONValue]?
    /// The argument rule the pair shows, if it was recorded for one.
    let rule: String?

    init(_ json: JSONValue) throws {
        guard let function = json["function"]?.string, let args = json["args"], let source = json["source"]?.string,
              let value = json["value"], let errors = json["errors"]?.array
        else { throw JSONError(offset: 0, reason: "not a recorded pair: \(json.compact)") }
        self.function = function
        self.args = args
        self.source = source
        self.value = value
        self.errors = errors
        parseErrors = json["parseErrors"]?.array ?? []
        zone = json["zone"]?.string
        draws = json["draws"]?.array
        rule = json["rule"]?.string
    }

    /// How a mismatch names the pair.
    var name: String {
        source + (zone.map { " in \($0)" } ?? "")
    }
}

/// The recording's tags (see intelligent-ui-functions.ts).
private enum Tag {
    case number(String), utf16, undefined

    init?(_ value: JSONValue) {
        guard case let .object(entries) = value, entries.count == 1, let entry = entries.first else { return nil }
        switch entry.key {
        case "$num": guard let text = entry.value.string else { return nil }; self = .number(text)
        case "$utf16": self = .utf16
        case "$undefined": self = .undefined
        default: return nil
        }
    }
}

/// Is this recording free of values that need the #160 or #161 boundary rules?
private func carried(_ value: JSONValue) -> Bool {
    var work = [value]
    while let next = work.popLast() {
        switch (Tag(next), next) {
        case (.number(let text)?, _): if text != "-0" { return false }
        case (.utf16?, _), (.undefined?, _): return false
        case (nil, .array(let items)): work.append(contentsOf: items)
        case (nil, .object(let entries)): work.append(contentsOf: entries.map(\.value))
        default: break
        }
    }
    return true
}

/// The recording preserves code units even in diagnostics. Repair only
/// unpaired surrogates, keeping adjacent valid pairs and all other text.
private func jsonResult(_ value: JSONValue) throws -> JSONValue {
    switch (Tag(value), value) {
    case (.number(let text)?, _): return text == "-0" ? .number(0) : .null
    case (.undefined?, _): return .null
    case (.utf16?, _):
        let units = try #require(value["$utf16"]?.array).map { unit in
            try #require(unit.number.flatMap(UInt16.init(exactly:)))
        }
        return .string(String(decoding: units, as: UTF16.self))
    case (nil, .array(let items)):
        return try .array(items.map(jsonResult))
    case (nil, .object(let entries)):
        return try .object(entries.compactMap { entry in
            if case .undefined? = Tag(entry.value) { return nil }
            return (entry.key, try jsonResult(entry.value))
        })
    default: return value
    }
}

/// Exact: compact JSON compared byte for byte, since Swift's `==` on strings
/// treats canonically equivalent text as equal.
private func same(_ a: JSONValue?, _ b: JSONValue?) -> Bool {
    // Pow and Payment follow Bun to the last bit. JSON writes both zeros as 0.
    if let x = a?.number, let y = b?.number {
        return x.bitPattern == y.bitPattern || (x == 0 && y == 0)
    }
    return a.map(\.compact).map { Array($0.utf8) } == b.map(\.compact).map { Array($0.utf8) }
}

private func runtimeError(_ issue: RuntimeIssue) -> JSONValue {
    var entries: [(key: String, value: JSONValue)] = [("source", .string("runtime")), ("code", .string(issue.code))]
    if let component = issue.component { entries.append(("component", .string(component))) }
    if let statementId = issue.statementId { entries.append(("statementId", .string(statementId))) }
    entries.append(("message", .string(issue.message)))
    if let hint = issue.hint { entries.append(("hint", .string(hint))) }
    return .object(entries)
}

/// One sequence per call. A callback runs on the session worker.
private final class RecordedDraws: @unchecked Sendable {
    private let lock = NSLock()
    private var remaining: ArraySlice<Double> = []
    private var extra = false

    func reset(_ values: [Double]) {
        lock.withLock {
            remaining = values[...]
            extra = false
        }
    }

    func next() -> Double {
        lock.withLock {
            guard let value = remaining.popFirst() else {
                extra = true
                return .nan
            }
            return value
        }
    }

    var exhaustedExactly: Bool { lock.withLock { remaining.isEmpty && !extra } }
}

@Suite @MainActor struct FunctionReplayTests {
    @Test(arguments: [1, 2, 1_024])
    func fieldIndexConversionsAreIndependentOfRowCount(_ rowCount: Int) throws {
        let args = JSONValue.object([("field", .string("0.00")), ("value", .number(1))])
        var conversions: [String: Int] = [:]
        let matches = try ListFunctions.matchField(args) { spelling in
            conversions[spelling, default: 0] += 1
            return Int(spelling)
        }
        let row = JSONValue.array([.array([.number(1)])])
        let rows = Array(repeating: row, count: rowCount)
        #expect(rows.filter(matches).isEmpty)
        #expect(conversions == ["0": 1, "00": 1])
        #expect(matches(.array([.object([("00", .number(1))])])))
    }

    @Test(arguments: [1, 2, 1_024])
    func inheritedMethodResolutionIsIndependentOfRowCount(_ rowCount: Int) {
        let field = String(repeating: "a", count: rowCount)
        var resolutions: [Bool] = []
        let read = ListFunctions.fieldReader("nested." + field, method: { name, array in
            #expect(name == field)
            resolutions.append(array)
            return .string(array ? "array method" : "object method")
        })
        for _ in 0..<rowCount {
            #expect(read(.object([("nested", .array([]))])) == .string("array method"))
            #expect(read(.object([("nested", .object([]))])) == .string("object method"))
        }
        #expect(read(.object([("nested", .object([(field, .string("own"))]))])) == .string("own"))
        #expect(resolutions.count == 2 && Set(resolutions) == [false, true])
    }

    @Test(arguments: [1_024, 2_048, 4_096])
    func literalSearchHasALinearComparisonBound(_ size: Int) {
        let prefix = String(repeating: "a", count: size / 2)
        let separator = prefix + "b"
        let nearMiss = prefix + prefix
        for (text, expected) in [(nearMiss, [nearMiss]), (nearMiss + "b", [prefix, ""])] {
            var comparisons = 0
            var parts: [String] = []
            forEachLiteralPart(of: text, separatedBy: separator, equal: { a, b in
                comparisons += 1
                return a == b
            }) { parts.append($0) }
            #expect(parts == expected)
            #expect(comparisons >= size)
            #expect(comparisons <= 2 * (text.utf16.count + separator.utf16.count))
        }
    }

    @Test func everyRecordedPairReplaysThroughTheRuntime() async throws {
        let pairs = try #require(recording["cases"]?.array).map(Pair.init)
        let declared = Set(IntelligentUI.functions.map(\.name))
        let implemented = Set(IntelligentUI.functions.filter { $0.callback != nil }.map(\.name))

        // Every function is declared, and the ones still to come are exactly the pending ones.
        #expect(declared.count == IntelligentUI.functions.count)
        #expect(declared.subtracting(implemented) == pending, "pending must list exactly the unimplemented functions")
        let unregistered = Set(pairs.map(\.function)).subtracting(declared)
        #expect(unregistered.isEmpty, "recorded but not registered: \(unregistered.sorted())")

        let draws = RecordedDraws()
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions(random: draws.next),
                                                     root: "Probe")
        let live = try LiveProgram(library: library)
        var mismatches: [String] = []
        var replayed: [String: Int] = [:]
        var projected: [String: Int] = [:]
        var signedZeroInputs: [String: Int] = [:]
        var refused: [String: Int] = [:]
        var refusedAfterUpstreamPruning: [String: Int] = [:]
        var refusedWithRequiredNull: [String: Int] = [:]
        var replayedRules = Set<String>()
        var waitingRules: [String: Int] = [:]
        for pair in pairs {
            // A pending function's call gives null, with no error from it.
            guard implemented.contains(pair.function) else {
                if let rule = pair.rule { waitingRules[rule, default: 0] += 1 }
                try await live.show(response: pair.source, streaming: false)
                if live.rootJSON?["props"]?["value"] != .null || !live.errors.isEmpty {
                    mismatches.append("\(pair.name): pending, but gave \(live.rootJSON?["props"]?["value"]?.compact ?? "no value") with \(live.errors.map(\.message))")
                }
                continue
            }
            guard pair.zone == nil else {
                mismatches.append("\(pair.name): needs a time zone")
                continue
            }
            let numbers = try (pair.draws ?? []).map { try #require($0.number) }
            let args = try #require(pair.args.array)
            // The parser removes the final optional literal in these pairs.
            // Only arguments that reach the call at runtime cross the JSON boundary.
            let runtimeArgs = pair.rule == "pruned optional literal" ? Array(args.dropLast()) : args
            let nonFiniteArgument = runtimeArgs.contains {
                if case .number(let text)? = Tag($0) { return text != "-0" }
                return false
            }
            // Bun can prune an invalid optional argument and still draw.
            // The host must refuse the call before consuming even its first draw.
            draws.reset(nonFiniteArgument ? [] : numbers)
            var expected = try jsonResult(pair.value)
            // JSON writes -0 as 0. These two functions expose the input sign in text.
            if ["Format", "Plural"].contains(pair.function), case .number("-0")? = args.first.flatMap(Tag.init) {
                signedZeroInputs[pair.function, default: 0] += 1
                let writtenArgs = try jsonResult(pair.args)
                let control = try #require(pairs.first { $0.function == pair.function && same($0.args, writtenArgs) })
                expected = try jsonResult(control.value)
            }
            if !nonFiniteArgument && !carried(.array([.array(runtimeArgs), pair.value, .array(pair.errors), .array(pair.parseErrors)])) {
                projected[pair.function, default: 0] += 1
                if !carried(.array(runtimeArgs)), ["Includes", "Join", "Split", "Replace"].contains(pair.function) {
                    // Repairing inputs can change the computed result. Bun also recorded
                    // this call with the arguments the Swift callback receives.
                    let writtenArgs = try jsonResult(.array(runtimeArgs))
                    let control = try #require(pairs.first { $0.function == pair.function && same($0.args, writtenArgs) })
                    expected = try jsonResult(control.value)
                }
            }
            let previousMismatchCount = mismatches.count
            let step = try await live.show(response: pair.source, streaming: false)
            if !draws.exhaustedExactly { mismatches.append("\(pair.name): wrong number of draws") }
            let value = live.rootJSON?["props"]?["value"]
            let errors = JSONValue.array(live.errors.map(runtimeError))
            if nonFiniteArgument {
                refused[pair.function, default: 0] += 1
                if pair.value != .null && !pair.errors.isEmpty {
                    refusedAfterUpstreamPruning[pair.function, default: 0] += 1
                }
                if args.contains(.null) && pair.value == .null && pair.errors.isEmpty {
                    refusedWithRequiredNull[pair.function, default: 0] += 1
                }
                if value != .null || live.errors.count != 1 || live.errors.first?.message.hasSuffix(" is not a finite number") != true {
                    mismatches.append("\(pair.name): expected null with a non-finite argument error, got \(value?.compact ?? "no value") with \(errors.compact)")
                }
            } else if !same(value, expected) {
                mismatches.append("\(pair.name): expected \(expected.compact), got \(value?.compact ?? "no value")")
            }
            if !nonFiniteArgument, !same(errors, try jsonResult(.array(pair.errors))) {
                mismatches.append("\(pair.name): expected errors \(JSONValue.array(pair.errors).compact), got \(errors.compact)")
            }
            let parseErrors = try JSONValue.parse(step.json)["meta"]?["errors"] ?? .array([])
            if !same(parseErrors, try jsonResult(.array(pair.parseErrors))) {
                mismatches.append("\(pair.name): expected parse errors \(JSONValue.array(pair.parseErrors).compact), got \(parseErrors.compact)")
            }
            if mismatches.count == previousMismatchCount {
                replayed[pair.function, default: 0] += 1
                if let rule = pair.rule { replayedRules.insert(rule) }
            }
        }
        #expect(mismatches == [])
        // Every implemented function has pairs that replayed.
        #expect(Set(replayed.keys) == implemented, "implemented without replayed pairs: \(implemented.subtracting(replayed.keys).sorted())")
        #expect(replayed == expectedReplayCounts)
        #expect(projected == jsonBoundaryPairs)
        #expect(signedZeroInputs == ["Format": 4, "Plural": 1])
        #expect(refused == topLevelNumberPairs)
        #expect(refusedAfterUpstreamPruning == ["Format": 6, "Plural": 3, "Join": 6, "Pad": 3, "Split": 3, "Duration": 3, "Random": 3])
        #expect(refusedWithRequiredNull == ["Format": 3, "Plural": 3, "Join": 3, "Pad": 3, "Split": 3, "Replace": 3, "Duration": 3, "Default": 3,
                                           "Pow": 3, "Random": 3, "Clamp": 3, "Percent": 3, "Lerp": 3,
                                           "Allocate": 3, "Payment": 3, "Convert": 3])
        #expect(waitingRules == pendingRulePairs)
        let uncovered = waitingRules.filter { !replayedRules.contains($0.key) }
        print("Intelligent UI argument rules recorded, not yet replayed: \(uncovered.sorted { $0.key < $1.key })")
        #expect(uncovered.isEmpty)
    }

    @Test(arguments: ["Append", "Prepend", "Concat", "Flatten", "Range"])
    func upstreamItemCapAccepts10000AndRejects10001(_ function: String) async throws {
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions, root: "Probe")
        let live = try LiveProgram(library: library)
        for size in [10_000, 10_001] {
            let prefix = "@Range(0, \(size - 1))"
            let call: String
            switch function {
            case "Concat": call = "@Concat(\(prefix), [0])"
            case "Flatten": call = "@Flatten([\(prefix), [0]])"
            case "Range": call = "@Range(0, \(size))"
            default: call = "@\(function)(\(prefix), 0)"
            }
            try await live.show(response: "root = Probe(\(call))", streaming: false)
            let value = live.rootJSON?["props"]?["value"]
            if size == 10_000 {
                #expect(value?.array?.count == size)
                #expect(live.errors.isEmpty)
            } else {
                #expect(value == .null)
                #expect(live.errors.map(\.message) == ["Evaluating prop \"value\" on Probe failed: @\(function) threw: more than 10000 items"])
            }
        }
    }

    @Test func formatTruncatesAndClampsDecimalPlacesToZeroThroughTen() async throws {
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions, root: "Probe")
        let live = try LiveProgram(library: library)
        for (places, expected) in [(-1.0, "$1"), (0, "$1"), (1.9, "$1.2"), (9, "$1.234567890"),
                                   (10, "$1.2345678902"), (11, "$1.2345678902")] {
            try await live.show(response: #"root = Probe(@Format(1.23456789015, "currency", \#(places)))"#, streaming: false)
            #expect(live.rootJSON?["props"]?["value"] == .string(expected))
            #expect(live.errors.isEmpty)
        }
    }

    @Test func formatAndPluralKeepTheSignWhenTheCallbackReceivesNegativeZero() throws {
        let pairs = try #require(recording["cases"]?.array).map(Pair.init).filter {
            guard ["Format", "Plural"].contains($0.function) else { return false }
            if case .number("-0")? = $0.args.array?.first.flatMap(Tag.init) { return true }
            return false
        }
        #expect(pairs.count == 5)
        for pair in pairs {
            let second = try #require(pair.args.array?.dropFirst().first)
            // Building the JSON with JSONValue.compact would already write the zero as 0.
            let args = pair.function == "Format"
                ? #"{"value":-0,"kind":\#(second.compact)}"#
                : #"{"n":-0,"word":\#(second.compact)}"#
            let callback = try #require(IntelligentUI.functions.first { $0.name == pair.function }?.callback)
            let result = try #require(try callback.call(argumentsJson: args))
            #expect(try same(JSONValue.parse(result), pair.value), "\(pair.name)")
        }
    }

    @Test func formatAndPluralUseUSSeparatorsUnderAFrenchNumericLocale() throws {
        // Change only this thread, and call the registered callbacks synchronously on it.
        let french = try #require(newlocale(LC_NUMERIC_MASK, "fr_FR.UTF-8", nil))
        defer { freelocale(french) }
        let previous = try #require(uselocale(french))
        defer { uselocale(previous) }
        #expect(String(cString: localeconv().pointee.decimal_point) == ",")
        for (name, json, expected) in [
            ("Format", #"{"value":12345.675,"kind":"currency","currency":"EUR"}"#, "€12,345.68"),
            ("Plural", #"{"n":12345.675,"word":"guest"}"#, "12,345.68 guests"),
        ] {
            let callback = try #require(IntelligentUI.functions.first { $0.name == name }?.callback)
            let result = try #require(try callback.call(argumentsJson: json))
            #expect(try JSONValue.parse(result) == .string(expected))
        }
    }

    @Test func padCapsTheRequestedLengthAt1000CodeUnits() async throws {
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions, root: "Probe")
        let live = try LiveProgram(library: library)
        for size in [999, 1000, 1001] {
            try await live.show(response: #"root = Probe(@Pad("😀", \#(size), "x"))"#, streaming: false)
            let expected = String(repeating: "x", count: min(size, 1000) - 2) + "😀"
            #expect(live.rootJSON?["props"]?["value"] == .string(expected))
            #expect(live.errors.isEmpty)
        }
        let alreadyLong = String(repeating: "😀", count: 501)
        try await live.show(response: #"root = Probe(@Pad("\#(alreadyLong)", 5000))"#, streaming: false)
        #expect(live.rootJSON?["props"]?["value"] == .string(alreadyLong))
        #expect(live.errors.isEmpty)
    }

    @Test func splitCapsTheNumberOfRetainedPartsAt10000() async throws {
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions, root: "Probe")
        let live = try LiveProgram(library: library)
        for size in [10_000, 10_001] {
            let text = "  " + String(repeating: "x", count: size)
            try await live.show(response: #"root = Probe(@Split("\#(text)", ""))"#, streaming: false)
            let value = live.rootJSON?["props"]?["value"]
            if size == 10_000 {
                #expect(value?.array?.count == size)
                #expect(live.errors.isEmpty)
            } else {
                #expect(value == .null)
                #expect(live.errors.map(\.message) == [#"Evaluating prop "value" on Probe failed: @Split threw: more than 10000 items"#])
            }
        }
    }

    @Test func joinWritesJavaScriptPropertyOrderAndEscapes() async throws {
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions, root: "Probe")
        let live = try LiveProgram(library: library)
        try await live.show(response: #"root = Probe(@Join([{"b": 1e-5, "10": "\b", "2": "\f", "01": "x"}]))"#,
                            streaming: false)
        #expect(live.rootJSON?["props"]?["value"] == .string(#"{"2":"\f","10":"\b","b":0.00001,"01":"x"}"#))
        #expect(live.errors.isEmpty)
    }

    @Test(arguments: ["Join", "Replace"])
    func expandedTextUsesTheExistingHostResultLimit(_ function: String) throws {
        let callback = try #require(IntelligentUI.functions.first { $0.name == function }?.callback)
        let repeated = JSONValue.string(String(repeating: "x", count: 4096))
        let args: JSONValue = function == "Join"
            ? .object([("list", .array(Array(repeating: .string(""), count: 4097))), ("sep", repeated)])
            : .object([("text", repeated), ("find", .string("x")), ("replacement", repeated)])
        #expect(throws: OpenUIFFI.FunctionError.Thrown(reason: "host function result byte limit")) {
            try callback.call(argumentsJson: args.compact)
        }
    }

    @Test func reverseWritesNullForNonFiniteNumbersInsideObjectsAndLists() async throws {
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions, root: "Probe")
        let live = try LiveProgram(library: library)
        try await live.show(response: "$a = [{n: 1e, xs: [-1e999]}, 1e999]\nroot = Probe(@Reverse($a))", streaming: false)
        #expect(live.rootJSON?["props"]?["value"] == .array([.null, .object([("n", .null), ("xs", .array([.null]))])]))
        #expect(live.errors.isEmpty)
    }

    @Test func allocateClampsRoundedDecimalPlacesToZeroThroughSix() async throws {
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions, root: "Probe")
        let live = try LiveProgram(library: library)
        for (places, expected) in [(-1.0, 1.0), (0, 1), (0.5, 1.2), (5.5, 1.234568), (6, 1.234568), (7, 1.234568)] {
            try await live.show(response: "root = Probe(@Allocate(1.23456789, [1], \(places)))", streaming: false)
            #expect(live.rootJSON?["props"]?["value"] == .array([.number(expected)]))
            #expect(live.errors.isEmpty)
        }
    }

    @Test(arguments: [1_000, 5_000, 10_000, 10_001])
    func allocateAwardsBonusesInLinearVisits(_ size: Int) {
        // 7919 is coprime to each size. The largest ranks win wherever they occur.
        let ranks = (0..<size).map { ($0 * 7919) % size }
        var visits = 0
        let remainders = ranks.lazy.map { rank in
            visits += 1
            // Adjacent doubles keep the cutoff ambiguous until the low bytes.
            return Double(bitPattern: Double(0.5).bitPattern + UInt64(rank))
        }
        let bonuses = size * 41 / 100
        var parts = Array(repeating: 0.0, count: size)
        awardAllocationBonuses(bonuses, remainders: remainders, parts: &parts)
        #expect(parts == ranks.map { $0 >= size - bonuses ? 1.0 : 0.0 })
        #expect(visits == 9 * size)
    }

    @Test func rangeUsesTheSameStepForExplicitNullAndOmission() async throws {
        let pairs = try #require(recording["cases"]?.array).map(Pair.init).filter {
            $0.function == "Range" && $0.rule == "optional null"
        }
        #expect(pairs.map(\.args) == [.array([.number(0), .number(3), .null]), .array([.number(0), .number(3)])])
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions, root: "Probe")
        let live = try LiveProgram(library: library)
        for pair in pairs {
            try await live.show(response: pair.source, streaming: false)
            #expect(live.rootJSON?["props"]?["value"] == .array([.number(0), .number(1), .number(2)]))
            #expect(live.errors.isEmpty)
        }
    }

    @Test func hostDrawsSelectBothEndsAndShuffleInDrawOrder() async throws {
        let draws = RecordedDraws()
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions(random: draws.next),
                                                     root: "Probe")
        let live = try LiveProgram(library: library)
        for (call, sequence, expected): (String, [Double], JSONValue) in [
            ("@Random(1, 3, true)", [0], .number(1)),
            ("@Random(3, 1, true)", [Double(1).nextDown], .number(3)),
            ("@Random(1, 5)", [0.25], .number(2)),
            ("@Pick([])", [], .null),
            ("@Pick([1, 2, 3])", [0], .number(1)),
            ("@Pick([1, 2, 3])", [Double(1).nextDown], .number(3)),
            ("@Shuffle([1, 2, 3])", [0, 0.75], .array([.number(3), .number(2), .number(1)])),
        ] {
            draws.reset(sequence)
            try await live.show(response: "root = Probe(\(call))", streaming: false)
            #expect(live.rootJSON?["props"]?["value"] == expected)
            #expect(live.errors.isEmpty)
            #expect(draws.exhaustedExactly)
        }
    }

    /// What a model writes to give t as Infinity: null and an error, never
    /// a made-up colour (upstream gives "#ffffff").
    @Test func anInfiniteTWrittenByAModelGivesNullAndAnError() async throws {
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions,
                                                     root: "Probe")
        let live = try LiveProgram(library: library)
        try await live.show(response: ##"root = Probe(@MixColor("#000000", "#ffffff", 1e308*10))"##, streaming: false)
        #expect(live.rootJSON?["props"]?["value"] == .null)
        #expect(live.errors.map(\.message) == [#"Evaluating prop "value" on Probe failed: @MixColor: t is not a finite number"#])
    }

    /// The pairs with t as Infinity, -Infinity or NaN can't cross the
    /// binding, so MixColor's body is checked against them directly.
    @Test func mixColorsBodyGivesUpstreamsResultForANonFiniteT() throws {
        let pairs = try #require(recording["cases"]?.array).map(Pair.init).filter {
            guard $0.function == "MixColor", let args = $0.args.array, args.count == 3,
                  case .number? = Tag(args[2]) else { return false }
            return args[2] != .object([("$num", .string("-0"))])
        }
        #expect(pairs.count == 3)
        for pair in pairs {
            let args = try #require(pair.args.array)
            guard let from = args.first?.string, let to = args.dropFirst().first?.string,
                  case let .number(text)? = args.last.flatMap(Tag.init) else { continue }
            let t = text == "NaN" ? Double.nan : text.hasPrefix("-") ? -Double.infinity : Double.infinity
            #expect(try .string(ColorFunctions.mixColor(from: from, to: to, t: t)) == pair.value, "\(pair.name)")
        }
    }
}
