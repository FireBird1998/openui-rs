// Color, Length, Radius and spacing against upstream's own helpers: the
// fixture's `cases` are upstream's fg, bg, size, radius and space from
// shared/props.ts, run on inputs taken from that source (every token, every
// word and character its CSS guard refuses, the whitespace JavaScript's
// trim and \s know, each unit size accepts, and the near misses of each).
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import SwiftTestSupport
import Testing

private struct Case {
    let input: JSONValue
    let output: String?
}

private func cases(_ helper: String) throws -> [Case] {
    let items = try #require(try themeFixture()["cases"]?[helper]?.array, "no \(helper) cases")
    return items.map { Case(input: $0["input"] ?? .null, output: $0["output"]?.string) }
}

/// Upstream's tokens.ts text for each token, to tell a token result from raw CSS.
private func tokenText() throws -> [String: String] {
    guard case let .object(entries)? = try themeFixture()["tokens"] else { return [:] }
    var out: [String: String] = [:]
    for (key, value) in entries { out[key] = value.string }
    return out
}

/// What upstream's fg or bg gave, in this library's terms.
private func expectedColor(_ output: String?, tokens: [String: String]) -> ColorValue? {
    guard let output else { return nil }
    if let name = tokens.first(where: { $0.value == output })?.key {
        guard let token = ColorToken(rawValue: name) else {
            Issue.record("upstream gave token \(name), which ColorToken lacks")
            return nil
        }
        return .token(token)
    }
    return .css(output)
}

/// What upstream's size gave (`12px`, `50%`, `auto`), in this library's terms.
private func expectedLength(_ output: String?) -> Length? {
    guard let output else { return nil }
    if output == "auto" { return .auto }
    for (unit, make) in [("px", Length.points), ("rem", Length.rem), ("em", Length.em), ("%", Length.percent),
                         ("vh", Length.vh), ("vw", Length.vw), ("fr", Length.fr)] as [(String, (Double) -> Length)]
        where output.hasSuffix(unit) {
        if let n = Double(output.dropLast(unit.count)) { return make(n) }
    }
    Issue.record("cannot read upstream's length \(output)")
    return nil
}

private func describe(_ v: JSONValue) -> String { v.compact }

@Suite(NativeTestWatchdog()) struct ValueRulesTests {
    @Test func colorAcceptsAndRejectsExactlyWhatUpstreamsFgDoes() throws {
        let tokens = try tokenText()
        let all = try cases("fg")
        #expect(all.count > 100)
        for c in all {
            #expect(ColorValue.foreground(c.input) == expectedColor(c.output, tokens: tokens),
                    "fg(\(describe(c.input)))")
        }
    }

    @Test func backgroundAcceptsAndRejectsExactlyWhatUpstreamsBgDoes() throws {
        let tokens = try tokenText()
        for c in try cases("bg") {
            #expect(ColorValue.background(c.input) == expectedColor(c.output, tokens: tokens),
                    "bg(\(describe(c.input)))")
        }
    }

    @Test func lengthAcceptsAndRejectsExactlyWhatUpstreamsSizeDoes() throws {
        let all = try cases("size")
        #expect(all.count > 50)
        for c in all {
            #expect(Length(c.input) == expectedLength(c.output), "size(\(describe(c.input)))")
        }
    }

    @Test func radiusAcceptsAndRejectsExactlyWhatUpstreamsRadiusDoes() throws {
        let tokens = try tokenText()
        for c in try cases("radius") {
            let expected: Radius?
            if let output = c.output, let name = tokens.first(where: { $0.value == output })?.key {
                let word = name.replacingOccurrences(of: "radius", with: "").lowercased()
                expected = try .token(#require(RadiusToken(rawValue: word)))
            } else if case let .points(p)? = expectedLength(c.output) {
                expected = .points(p)
            } else {
                expected = nil
            }
            #expect(Radius(c.input) == expected, "radius(\(describe(c.input)))")
        }
    }

    @Test func overflowingSpacingDropsTheDeclaration() {
        #expect(Spacing.points(JSONValue.number(4.5e307)) == nil)
        #expect(Spacing.points(JSONValue.number(4e307)) == 1.6e308)
    }

    @Test func spacingIsUpstreamsFourPointSteps() throws {
        for c in try cases("space") {
            let expected = expectedLength(c.output).flatMap { if case let .points(p) = $0 { p } else { nil } }
            #expect(Spacing.points(c.input) == expected, "space(\(describe(c.input)))")
        }
    }

    @Test func aPropValueReadsAsItsScalarAndAnythingElseIsDropped() {
        #expect(ColorValue.foreground(PropValue.scalar(.string("ink"))) == .token(.ink))
        #expect(ColorValue.background(PropValue.scalar(.string("danger"))) == .token(.dangerSoft))
        #expect(ColorValue.foreground(PropValue.expression(.string("ink"))) == nil)
        #expect(ColorValue.foreground(PropValue.list([.scalar(.string("ink"))])) == nil)
        #expect(ColorValue.foreground(nil as PropValue?) == nil)
        #expect(Length(PropValue.scalar(.number(-4))) == .points(0))
        #expect(Length(PropValue.scalar(.string("50%"))) == .percent(50))
        #expect(Radius(PropValue.scalar(.string("lg"))) == .token(.lg))
        #expect(Spacing.points(PropValue.scalar(.number(3))) == 12)
    }

    @Test func aLengthTooLongForADoubleStaysFinite() {
        let digits = String(repeating: "9", count: 400)
        #expect(Length(.string(digits)) == .points(.greatestFiniteMagnitude))
        #expect(Length(.string(digits + "%")) == .percent(.greatestFiniteMagnitude))
    }
}
