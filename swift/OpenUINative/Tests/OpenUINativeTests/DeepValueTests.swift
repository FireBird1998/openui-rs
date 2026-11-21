// Deep values sent to the core (#148's fix round). `compact` and `==` walk
// an explicit stack, so a value nested deeper than a small thread's stack
// can take, built by the app or parsed (a remote tool's schema, a tool
// provider's result), is written or compared without overflowing. The
// async calls that write it run on the cooperative pool, whose threads have
// small stacks; before the fix a 511-level schema parsed by `JSONValue.parse`
// crashed a debug build there (signal 10) at 440 levels and more.
import Foundation
@testable import OpenUINative
import Testing

/// Holds a deep value so the test controls where its last reference goes.
private final class Holder: @unchecked Sendable {
    var value: JSONValue?
    init(_ value: JSONValue) { self.value = value }
}

/// Releases a deep value on a thread with a large stack. Swift releases a
/// nested enum recursively, so dropping 100,000 levels needs far more stack
/// than the cooperative pool's threads have (see the README).
private func releaseOnLargeStack(_ holder: Holder) async {
    await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
        let thread = Thread {
            holder.value = nil
            done.resume()
        }
        thread.stackSize = 1 << 29
        thread.start()
    }
}

private func nestedArrays(_ depth: Int) -> JSONValue {
    var value = JSONValue.number(1)
    for _ in 0..<depth { value = .array([value]) }
    return value
}

private func nestedObjects(_ depth: Int) -> JSONValue {
    var value = JSONValue.number(1)
    for _ in 0..<depth { value = .object([("x", value)]) }
    return value
}

/// `compact` as it was written before the fix: recursive. Used only to
/// check the iterative one writes the same text, on shallow values.
private func recursiveCompact(_ value: JSONValue) -> String {
    switch value {
    case .null: "null"
    case let .bool(b): b ? "true" : "false"
    case let .number(n): JSONValue.format(n)
    case let .string(s): quoted(s)
    case let .array(a): "[" + a.map(recursiveCompact).joined(separator: ",") + "]"
    case let .object(o): "{" + o.map { quoted($0.key) + ":" + recursiveCompact($0.value) }.joined(separator: ",") + "}"
    }
}

/// `==` as it was written before the fix. Its arrays were compared with
/// `Array ==`, which answers true at once for two empty arrays or two
/// sharing one buffer, before comparing elements (`arrayEqualityShortcut`
/// checks the standard library still does).
private func recursiveEqual(_ a: JSONValue, _ b: JSONValue) -> Bool {
    switch (a, b) {
    case (.null, .null): true
    case let (.bool(x), .bool(y)): x == y
    case let (.number(x), .number(y)): x == y
    case let (.string(x), .string(y)): x == y
    case let (.array(x), .array(y)):
        x.count == y.count && (x.isEmpty || oneBuffer(x, y) || zip(x, y).allSatisfy { recursiveEqual($0, $1) })
    case let (.object(x), .object(y)):
        x.count == y.count && zip(x, y).allSatisfy { $0.key == $1.key && recursiveEqual($0.value, $1.value) }
    default: false
    }
}

private func quoted(_ s: String) -> String { JSONValue.quote(s) }

private func oneBuffer<T>(_ a: [T], _ b: [T]) -> Bool {
    a.withUnsafeBufferPointer { x in b.withUnsafeBufferPointer { y in x.baseAddress == y.baseAddress } }
}

/// `PropValue.summary` as it was before it became iterative.
private func recursiveSummary(_ value: PropValue) -> String {
    switch value {
    case let .scalar(json): json.compact
    case let .expression(json): "ƒ " + json.compact
    case let .binding(marker): "⇄ " + marker.target
    case let .plan(plan): "▶ action, \(plan.steps.count) step" + (plan.steps.count == 1 ? "" : "s")
    case let .list(items): "[" + items.map(recursiveSummary).joined(separator: ", ") + "]"
    case let .record(fields): "{" + fields.map { $0.name + ": " + recursiveSummary($0.value) }.joined(separator: ", ") + "}"
    case let .element(element): element.typeName + "(…)"
    }
}

/// `PropValue.holdsElements` as it was before it became iterative.
private func recursiveHoldsElements(_ value: PropValue) -> Bool {
    switch value {
    case .element: true
    case let .list(items): items.contains { recursiveHoldsElements($0) }
    case let .record(fields): fields.contains { recursiveHoldsElements($0.value) }
    case .expression, .binding, .plan, .scalar: false
    }
}

/// Runs `body` on a thread with a large stack and waits for it.
private func onLargeStack<T: Sendable>(_ body: @escaping @Sendable () -> T) async -> T {
    await withCheckedContinuation { (done: CheckedContinuation<T, Never>) in
        let thread = Thread { done.resume(returning: body()) }
        thread.stackSize = 1 << 26
        thread.start()
    }
}

/// JSON shaped like each kind of prop value `PropValue(json:)` tells apart.
private func randomPropJSON(_ rng: inout SplitMix, depth: Int) -> JSONValue {
    let leaf = depth > 5
    switch rng.below(leaf ? 3 : 9) {
    case 0: return .null
    case 1: return rng.below(2) == 0 ? .number(Double(rng.below(1000)) / 8) : .bool(true)
    case 2: return .string(rng.below(4) == 0 ? String(repeating: "long ", count: 200) : awkwardStrings[rng.below(awkwardStrings.count)])
    case 3: return .array((0..<rng.below(5)).map { _ in randomPropJSON(&rng, depth: depth + 1) })
    case 4: return .object((0..<rng.below(4)).map { _ in
        (key: awkwardStrings[rng.below(awkwardStrings.count)], value: randomPropJSON(&rng, depth: depth + 1)) })
    case 5: return .object([("type", .string("element")), ("typeName", .string(["Card", "TextContent", "Stack"][rng.below(3)])),
                            ("props", .object([("children", randomPropJSON(&rng, depth: depth + 1))])),
                            ("partial", .bool(false)), ("hasDynamicProps", .bool(rng.below(2) == 0))])
    case 6: return .object([("__reactive", .string("assign")), ("target", .string("$v\(rng.below(9))")),
                            ("expr", .object([("k", .string("StateRef")), ("n", .string("$value"))]))])
    case 7: return .object([("steps", .array((0..<rng.below(4)).map { _ in .object([("type", .string("set"))]) }))])
    default: return .object([("k", .string(["Ref", "BinOp", "Str"][rng.below(3)])), ("n", .string("x"))])
    }
}

/// A small deterministic generator, so a failure names its seed.
private struct SplitMix {
    var state: UInt64
    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
    mutating func below(_ n: Int) -> Int { Int(next() % UInt64(n)) }
}

private let awkwardStrings = ["", "a", "\"", "\\", "\n\r\t", "\u{0}\u{1}\u{1f}", "\u{7f}", "é", "😀", "\u{2028}\u{2029}",
                              "</script>", "key with spaces", "\u{FFFD}", "日本語"]
private let awkwardNumbers: [Double] = [0, -0.0, 1, -1, 0.1, 1e-7, 1e15, 1e15 - 1, -1e15 + 1, 1e21, 123456789.125,
                                        Double(Int64.max), -Double(Int64.max), .leastNonzeroMagnitude, .greatestFiniteMagnitude, 2.5e-308]

private func randomValue(_ rng: inout SplitMix, depth: Int) -> JSONValue {
    switch rng.below(depth > 6 ? 4 : 6) {
    case 0: return .null
    case 1: return .bool(rng.below(2) == 0)
    case 2: return .number(rng.below(3) == 0 ? awkwardNumbers[rng.below(awkwardNumbers.count)]
                           : Double(Int(rng.next() % 2_000_001) - 1_000_000) / Double(1 + rng.below(1000)))
    case 3: return .string(awkwardStrings[rng.below(awkwardStrings.count)])
    case 4: return .array((0..<rng.below(5)).map { _ in randomValue(&rng, depth: depth + 1) })
    default:
        return .object((0..<rng.below(5)).map { _ in
            (key: awkwardStrings[rng.below(awkwardStrings.count)], value: randomValue(&rng, depth: depth + 1))
        })
    }
}

@Suite struct DeepValueTests {
    /// The reviewer's case: a schema `parse` accepts, 511 levels deep, as a
    /// tool's input schema. The typed call ends as the raw spec call with
    /// the same text ends, refused by the core, not in a crash.
    @Test func aParsedSchemaAsDeepAsParseAllowsIsRefusedLikeTheRawSpec() async throws {
        let core = Core()
        let schemaText = String(repeating: "{\"x\":", count: 511) + "1" + String(repeating: "}", count: 511)
        let deep = try JSONValue.parse(schemaText)
        let library = JSONValue.object([("components", .object([]))])
        let typed = await #expect(throws: OpenUIError.self) {
            try await core.systemPrompt(library: library,
                options: PromptOptions(tools: [.spec(ToolSpec(name: "t", inputSchema: deep))]))
        }
        let raw = await #expect(throws: OpenUIError.self) {
            try await core.systemPrompt(spec: #"{"library":{"components":{}},"promptOptions":{"tools":[{"name":"t","inputSchema":"#
                + schemaText + "}]}}")
        }
        #expect(typed != nil && typed == raw)
        #expect(typed?.kind == .invalidPromptSpec)
        // The same for a library that deep in cloud mode.
        let cloud = await #expect(throws: OpenUIError.self) { try await core.cloudConfigBlock(library: deep) }
        let rawCloud = await #expect(throws: OpenUIError.self) {
            try await core.systemPrompt(spec: #"{"cloud":true,"library":"# + schemaText + #","promptOptions":{}}"#)
        }
        #expect(cloud != nil && cloud == rawCloud)
    }

    @Test(arguments: [false, true])
    func compactWritesValuesFarDeeperThanParseAllows(objects: Bool) async {
        let depth = 100_000
        let holder = Holder(objects ? nestedObjects(depth) : nestedArrays(depth))
        let text = holder.value?.compact
        let open = objects ? "{\"x\":" : "["
        let close = objects ? "}" : "]"
        #expect(text == String(repeating: open, count: depth) + "1" + String(repeating: close, count: depth))
        await releaseOnLargeStack(holder)
    }

    @Test func equalityComparesValuesFarDeeperThanParseAllows() async {
        let depth = 100_000
        let a = Holder(nestedObjects(depth)), b = Holder(nestedObjects(depth)), c = Holder(nestedArrays(depth))
        let same = a.value == b.value
        let different = a.value == c.value
        #expect(same && !different)
        await releaseOnLargeStack(a)
        await releaseOnLargeStack(b)
        await releaseOnLargeStack(c)
    }

    /// The standard library's `Array ==` answers true for two arrays sharing
    /// one buffer without comparing elements; the old `==` relied on it.
    @Test func arrayEqualityShortcut() {
        let nan = [Double.nan]
        let copy = nan
        #expect(nan == copy)
        #expect([Double.nan] != [Double.nan])
    }

    /// A value holding NaN compared with itself, with a copy, and with a
    /// fresh value built the same way, alone and nested in arrays and
    /// objects: the iterative `==` answers as the recursive one did.
    @Test func equalityOnValuesHoldingNaNAgreesWithTheRecursiveOne() {
        func shapes() -> [JSONValue] {
            let nan = JSONValue.number(.nan)
            return [
                nan, .array([nan]), .array([.number(1), nan]), .object([("a", nan)]), .array([.array([nan])]),
                .array([.object([("a", nan)])]), .object([("a", .array([nan]))]), .object([("a", .object([("b", nan)]))]),
                .array([.array([]), nan]), .object([("a", .array([nan])), ("b", nan)]),
                .array([.number(.infinity), .number(-.infinity), nan]),
            ]
        }
        let values = shapes(), fresh = shapes()
        var disagreements = 0
        var compared = 0
        for (i, value) in values.enumerated() {
            let copy = value
            let wrappedInArray = JSONValue.array([value]), wrappedInObject = JSONValue.object([("w", value)])
            let wrappedCopy = wrappedInArray
            for (a, b) in [(value, value), (value, copy), (value, fresh[i]), (wrappedInArray, wrappedInArray),
                           (wrappedInArray, wrappedCopy), (wrappedInObject, wrappedInObject),
                           (wrappedInArray, .array([fresh[i]])), (wrappedInObject, .object([("w", fresh[i])]))] {
                compared += 1
                if (a == b) != recursiveEqual(a, b) { disagreements += 1 }
            }
        }
        #expect(compared == values.count * 8)
        #expect(disagreements == 0)
        // The case that changed in the first fix: an array holding NaN equals itself.
        let nanArray = JSONValue.array([.number(.nan)])
        #expect(nanArray == nanArray)
        #expect(JSONValue.array([.number(.nan)]) != JSONValue.array([.number(.nan)]))
    }

    /// The reviewer's program: a list as deep as the core allows, decoded
    /// from a one-shot parse, summarised off the main actor (the
    /// cooperative pool's small stack). The recursive `summary` crashed a
    /// debug build here.
    @Test func aListAtTheCoresDepthLimitIsSummarisedOffTheMainActor() async throws {
        let core = Core()
        let library = try await core.library(schema: #"{"$defs":{"Stack":{"properties":{"children":{"type":"array"}}},"TextContent":{"properties":{"text":{}}}}}"#)
        let step = try await core.parseOneShot("root = TextContent(" + String(repeating: "[", count: 255) + "1"
            + String(repeating: "]", count: 255) + ")", library: library)
        #expect(step.result.depthLimited)
        let value = try #require(step.result.root?.props.first?.value)
        let summary = value.summary
        #expect(!value.holdsElements)
        let old = await onLargeStack { recursiveSummary(value) }
        #expect(summary == old)
        #expect(summary.count == 514)
    }

    /// The iterative `summary` and `holdsElements` give what the recursive
    /// ones gave: on 5,000 generated prop values of every kind, and on every
    /// prop of the 152 recorded react-ui programs.
    @Test func theIterativePropWalksAgreeWithTheRecursiveOnes() async throws {
        var rng = SplitMix(state: 1480)
        var checked = 0
        for _ in 0..<5_000 {
            let value = PropValue(json: randomPropJSON(&rng, depth: 0))
            #expect(value.summary == recursiveSummary(value))
            #expect(value.holdsElements == recursiveHoldsElements(value))
            checked += 1
        }
        var directory = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { directory.deleteLastPathComponent() }
        let conformance = directory.appendingPathComponent("conformance")
        let core = Core()
        let library = try await core.library(schema: String(contentsOf: conformance.appendingPathComponent("imported/schemas/react-ui.json"), encoding: .utf8))
        let manifest = try JSONValue.parse(String(contentsOf: conformance.appendingPathComponent("recorded.json"), encoding: .utf8))
        var programs = 0
        for entry in manifest["cases"]?.array ?? [] {
            guard entry["schema"]?.string == "react-ui", let file = entry["file"]?.string else { continue }
            let program = try String(contentsOf: conformance.appendingPathComponent(file), encoding: .utf8)
            guard let root = try await core.parseOneShot(program, library: library).result.root else { continue }
            programs += 1
            var elements = [root]
            while let element = elements.popLast() {
                var values = element.props.map(\.value)
                while let value = values.popLast() {
                    #expect(value.summary == recursiveSummary(value))
                    #expect(value.holdsElements == recursiveHoldsElements(value))
                    checked += 1
                    switch value {
                    case let .element(child): elements.append(child)
                    case let .list(items): values += items
                    case let .record(fields): values += fields.map(\.value)
                    default: break
                    }
                }
            }
        }
        #expect(programs >= 100)
        #expect(checked > 5_000)
    }

    /// The iterative `compact` and `==` give what the recursive ones gave,
    /// on hand-picked awkward values and 5,000 generated ones.
    @Test func theIterativeWalksAgreeWithTheRecursiveOnes() throws {
        var values: [JSONValue] = [
            .null, .bool(true), .bool(false), .array([]), .object([]), .array([.array([]), .object([])]),
            .object([("", .null), ("", .number(1))]), .object([("b", .number(1)), ("a", .number(2))]),
        ]
        values += awkwardNumbers.map(JSONValue.number) + awkwardStrings.map(JSONValue.string)
        values.append(.object(awkwardStrings.map { (key: $0, value: .string($0)) }))
        var directory = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { directory.deleteLastPathComponent() }
        let reactUI = directory.appendingPathComponent("conformance/prompts/react-ui-schema.json")
        values.append(try JSONValue.parse(String(contentsOf: reactUI, encoding: .utf8)))
        var rng = SplitMix(state: 148)
        for _ in 0..<5_000 { values.append(randomValue(&rng, depth: 0)) }
        for (i, value) in values.enumerated() {
            #expect(value.compact == recursiveCompact(value), "value \(i)")
            let other = values[(i * 7 + 3) % values.count]
            #expect((value == other) == recursiveEqual(value, other), "values \(i) and \((i * 7 + 3) % values.count)")
            #expect(value == value)
            // Text read back gives an equal value, for values whose numbers
            // and strings survive the round trip.
            if let back = try? JSONValue.parse(value.compact), recursiveEqual(back, value) { #expect(back == value) }
        }
    }
}
