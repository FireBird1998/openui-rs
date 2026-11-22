// Upstream's own function results, replayed through the Rust runtime with
// this library's Swift functions registered. reference/intelligent-ui-functions.ts
// records them (scripts/regen-golden.sh runs it) from upstream's function
// source vendored at the library pin, through the vendored parser and
// evaluator: conformance/golden/intelligent-ui-functions.json. Each pair is
// a program (`$a0 = …`, `root = Probe(@Name($a0, …))`) and what upstream
// evaluated the probe's value to, with its parse and evaluation errors.
//
// Every recorded function must be registered: implemented, or listed in
// `pending` until its ticket lands. A pending function's pairs only check
// that its call gives null.
import Foundation
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import Testing

/// Pairs a host function can't receive, by function: recorded, not replayed
/// (CONFORMANCE.md, "Intelligent UI function arguments JSON can't carry").
/// Each must still give null with an error. A new one fails the test until
/// someone looks.
private let uncarriedPairs: [String: Int] = [
    // t as Infinity, -Infinity and NaN, and a lone surrogate in from.
    "MixColor": 4,
]

/// Argument-rule pairs whose functions are still pending: recorded, not yet
/// replayed. MixColor has no optional or array params, so "optional null"
/// and "pruned array items" wait for their functions (#161 proves both).
private let rulesNotYetReplayed: [String: Int] = [
    "optional null": 3, "pruned array items": 1, "invalid optional word": 2, "required any": 3,
]

/// Declared, returning null until their tickets land.
private let pending: Set<String> = [
    // lists.ts
    "Append", "Prepend", "Concat", "Remove", "RemoveAt", "Update", "Toggle", "Includes", "Find", "IndexOf",
    "Take", "Slice", "Unique", "Reverse", "Range", "Pick", "Shuffle", "Flatten", "Chunk",
    // numbers.ts
    "Pow", "Sqrt", "Random", "Clamp", "Percent", "Lerp", "Allocate", "Payment", "Convert",
    // text.ts
    "Format", "Join", "Plural", "Pad", "Upper", "Lower", "Trim", "Capitalize", "Split", "Replace", "Duration",
    "Default",
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
        "@\(function)(\(args.array?.map(\.compact).joined(separator: ", ") ?? args.compact))" + (zone.map { " in \($0)" } ?? "")
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

/// Whether a host function could receive these arguments. The runtime hands
/// a function its arguments as JSON, as `JSON.stringify` writes them, so
/// Infinity and NaN arrive as null; and a Swift string can't hold a lone
/// surrogate. Upstream's results for those are recorded but can't be replayed.
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

/// A recorded result as the runtime's tree carries it to Swift: JSON, as
/// `JSON.stringify` writes it (-0 as 0, Infinity and NaN as null, undefined
/// array items as null and undefined fields left out). Nil for a lone
/// surrogate, which no Swift string holds.
private func carriedResult(_ value: JSONValue) -> JSONValue? {
    switch (Tag(value), value) {
    case (.number(let text)?, _): return text == "-0" ? .number(0) : .null
    case (.undefined?, _): return .null
    case (.utf16?, _): return nil
    case (nil, .array(let items)):
        var out: [JSONValue] = []
        for item in items {
            guard let item = carriedResult(item) else { return nil }
            out.append(item)
        }
        return .array(out)
    case (nil, .object(let entries)):
        var out: [(key: String, value: JSONValue)] = []
        for entry in entries {
            if case .undefined? = Tag(entry.value) { continue }
            guard let item = carriedResult(entry.value) else { return nil }
            out.append((entry.key, item))
        }
        return .object(out)
    default: return value
    }
}

/// Exact: compact JSON compared byte for byte, since Swift's `==` on strings
/// treats canonically equivalent text as equal.
private func same(_ a: JSONValue?, _ b: JSONValue?) -> Bool {
    a.map(\.compact).map { Array($0.utf8) } == b.map(\.compact).map { Array($0.utf8) }
}

private func runtimeError(_ issue: RuntimeIssue) -> JSONValue {
    var entries: [(key: String, value: JSONValue)] = [("source", .string("runtime")), ("code", .string(issue.code))]
    if let component = issue.component { entries.append(("component", .string(component))) }
    if let statementId = issue.statementId { entries.append(("statementId", .string(statementId))) }
    entries.append(("message", .string(issue.message)))
    if let hint = issue.hint { entries.append(("hint", .string(hint))) }
    return .object(entries)
}

@Suite @MainActor struct FunctionReplayTests {
    @Test func everyRecordedPairReplaysThroughTheRuntime() async throws {
        let pairs = try #require(recording["cases"]?.array).map(Pair.init)
        let declared = Set(IntelligentUI.functions.map(\.name))
        let implemented = Set(IntelligentUI.functions.filter { $0.callback != nil }.map(\.name))

        // Every function is declared, and the ones still to come are exactly the pending ones.
        #expect(declared.count == IntelligentUI.functions.count)
        #expect(declared.subtracting(implemented) == pending, "pending must list exactly the unimplemented functions")
        let unregistered = Set(pairs.map(\.function)).subtracting(declared)
        #expect(unregistered.isEmpty, "recorded but not registered: \(unregistered.sorted())")

        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions,
                                                     root: "Probe")
        let live = try LiveProgram(library: library)
        var mismatches: [String] = []
        var replayed: [String: Int] = [:]
        var uncarried: [String: Int] = [:]
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
            // What the function receives isn't what upstream's did, so the
            // only promise is an error and null, never a made-up value.
            guard carried(pair.args) else {
                uncarried[pair.function, default: 0] += 1
                try await live.show(response: pair.source, streaming: false)
                if live.rootJSON?["props"]?["value"] != .null || live.errors.isEmpty {
                    mismatches.append("\(pair.name): not carried, but gave \(live.rootJSON?["props"]?["value"]?.compact ?? "no value") with \(live.errors.map(\.message))")
                }
                continue
            }
            // Functions that read the time zone, the clock or random draws
            // take them from a source the replay supplies; none does yet.
            guard pair.zone == nil, pair.draws == nil else {
                mismatches.append("\(pair.name): needs a time zone or a random source the replay doesn't supply yet")
                continue
            }
            guard let expected = carriedResult(pair.value) else {
                mismatches.append("\(pair.name): upstream gives a lone surrogate, which no Swift string holds")
                continue
            }
            let step = try await live.show(response: pair.source, streaming: false)
            let value = live.rootJSON?["props"]?["value"]
            if !same(value, expected) {
                mismatches.append("\(pair.name): expected \(expected.compact), got \(value?.compact ?? "no value")")
            }
            let errors = JSONValue.array(live.errors.map(runtimeError))
            if !same(errors, carriedResult(.array(pair.errors))) {
                mismatches.append("\(pair.name): expected errors \(JSONValue.array(pair.errors).compact), got \(errors.compact)")
            }
            let parseErrors = try JSONValue.parse(step.json)["meta"]?["errors"] ?? .array([])
            if !same(parseErrors, .array(pair.parseErrors)) {
                mismatches.append("\(pair.name): expected parse errors \(JSONValue.array(pair.parseErrors).compact), got \(parseErrors.compact)")
            }
            replayed[pair.function, default: 0] += 1
        }
        #expect(mismatches == [])
        // Every implemented function has pairs that replayed.
        #expect(Set(replayed.keys) == implemented, "implemented without replayed pairs: \(implemented.subtracting(replayed.keys).sorted())")
        #expect(uncarried == uncarriedPairs, "pairs a host function can't receive, by function")
        print("Intelligent UI argument rules recorded, not yet replayed: \(waitingRules.sorted { $0.key < $1.key })")
        #expect(waitingRules == rulesNotYetReplayed, "argument-rule pairs recorded, not yet replayed")
    }

    /// What a model writes to give t as Infinity: null and an error, never
    /// a made-up colour (upstream gives "#ffffff").
    @Test func anInfiniteTWrittenByAModelGivesNullAndAnError() async throws {
        let library = try await Core().createLibrary(components: [probe], functions: IntelligentUI.functions,
                                                     root: "Probe")
        let live = try LiveProgram(library: library)
        try await live.show(response: ##"root = Probe(@MixColor("#000000", "#ffffff", 1e308*10))"##, streaming: false)
        #expect(live.rootJSON?["props"]?["value"] == .null)
        #expect(live.errors.map(\.message) == [#"Evaluating prop "value" on Probe failed: @MixColor threw: t is not a number"#])
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
