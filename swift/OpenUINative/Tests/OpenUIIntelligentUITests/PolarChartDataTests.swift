import Foundation
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import SwiftTestSupport
import Testing

@Suite(NativeTestWatchdog()) struct PolarChartDataTests {
    @Test func slicesUseFirstSeriesAndKeepSortedRowsAndPalette() throws {
        let data = try chart([
            row("third", 3), row("last", 0), row("tie first", 9), row("fifth", 1),
            row("tie second", 9), row("sixth", -2), row("fourth", 2), row("second", 7),
        ], series: ["amount", "other"])
        #expect(data.slices.map(\.label.text) == [
            "tie first", "tie second", "second", "third", "fourth", "fifth", "last", "sixth",
        ])
        #expect(data.slices.map(\.value) == [9, 9, 7, 3, 2, 1, 0, -2])
        #expect(data.slices.map(\.colourIndex) == [0, 1, 2, 3, 4, 5, 0, 1])
    }

    @Test func defaultsUseFirstObjectAndExplicitCategoryAndSeriesAreIndependent() throws {
        let rows: [JSONValue] = [.string("skip"), .object([
            ("2", .number(3)), ("01", .string("category")), ("1", .number(8)), ("amount", .number(99)),
        ]), .object([("01", .number(7)), ("1", .string("0x10")), ("2", .number(4))]), .object([])]
        let inferred = try chart(rows)
        #expect(inferred.slices.map(\.label.text) == ["7", "category", "undefined"])
        #expect(inferred.slices.map(\.value) == [16, 8, 0])
        let explicit = try chart(rows, x: "1", series: ["2", "amount"])
        #expect(explicit.slices.map(\.label.text) == ["0x10", "8", "undefined"])
        #expect(explicit.slices.map(\.value) == [4, 3, 0])
        let noSeries = try chart([.object([("name", .string("A")), ("", .string("12"))])])
        #expect(noSeries.slices.first?.value == 12)
    }

    @Test func numberCoercionPreservesInfinitiesAndZerosInvalidOrMissingValues() throws {
        let data = try chart([
            .object([("name", .string("missing"))]),
            row("nan", .string("NaN")), row("negative infinity", .string("-Infinity")),
            row("signed", .string(" -2.5 ")), row("infinity first", .string("Infinity")),
            row("blank", .string("  ")), row("infinity second", .string("+Infinity")),
            row("number", 4),
        ], series: ["amount"], percent: true)
        #expect(data.slices.map(\.label.text) == [
            "infinity first", "infinity second", "number", "missing", "nan", "blank", "signed", "negative infinity",
        ])
        #expect(data.slices.map(\.value) == [.infinity, .infinity, 4, 0, 0, 0, -2.5, -.infinity])
        let total = data.visibleTotal(hiding: [])
        #expect(total.isNaN)
        let first = try #require(data.slices.first)
        #expect(data.legendValue(first, total: total) == "0.0%")
        #expect(data.tooltipValue(first, total: total, locale: Locale(identifier: "en_US")) == "NaN%")
    }

    @Test func malformedPrefixedNumberDoesNotBecomePositiveInfinityAfterOverflow() throws {
        let long = "0x" + String(repeating: "f", count: 300)
        let data = try chart([row("bad", .string(long + "z")), row("valid", .string(long))], series: ["amount"])
        #expect(data.slices.map(\.value) == [.infinity, 0])
    }

    @Test func percentTooltipsAndLegendSharesUseTheVisibleTotal() throws {
        let rows = [row("small", 1), row("large", 3)]
        let percent = try chart(rows, percent: true)
        let total = percent.visibleTotal(hiding: [])
        let locale = Locale(identifier: "en_US")
        #expect(percent.slices.map { percent.tooltipValue($0, total: total, locale: locale) } == ["75.0%", "25.0%"])
        #expect(percent.slices.map { percent.legendValue($0, total: total) } == ["75.0%", "25.0%"])
        let hidden = percent.toggling(.init("small"), in: [])
        let large = try #require(percent.slices.first)
        #expect(percent.legendValue(large, total: percent.visibleTotal(hiding: hidden)) == "100.0%")
        #expect(percent.toggling(.init("large"), in: hidden) == hidden)

        let number = try chart([row("large", 123_456), row("small", 1_234.5678)])
        #expect(number.slices.map { number.tooltipValue($0, total: 0, locale: locale) } == ["123.4K", "1,234.568"])
        let thirds = try chart([row("A", 1), row("B", 1), row("C", 1)], percent: true)
        #expect(thirds.slices.map { thirds.legendValue($0, total: 3) } == ["33.3%", "33.3%", "33.3%"])
        let rounding = try chart([row("A", 1)])
        let a = try #require(rounding.slices.first)
        #expect(rounding.legendValue(a, total: 80) == "1.3%")
    }

    @Test func zeroAndNegativeSharesFollowDifferentLegendAndTooltipRules() throws {
        let data = try chart([row("positive", 4), row("zero", 0), row("negative", -2)], percent: true)
        let locale = Locale(identifier: "en_US")
        let total = data.visibleTotal(hiding: [])
        #expect(data.slices.map { data.legendValue($0, total: total) } == ["200.0%", "0.0%", "-100.0%"])
        #expect(data.slices.map { data.tooltipValue($0, total: 0, locale: locale) } == ["0%", "0%", "0%"])
        #expect(data.slices.map { data.legendValue($0, total: -2) } == ["0.0%", "0.0%", "0.0%"])
        #expect(data.slices.map { data.tooltipValue($0, total: -2, locale: locale) } == ["-200.0%", "0.0%", "100.0%"])
    }

    @Test func repeatedLabelsShareTogglesAndKeepUpstreamsLastItemGuard() throws {
        let data = try chart([row("A", 3), row("A", 2), row("B", 1)])
        let hidden = data.toggling(.init("A"), in: [])
        #expect(data.visibleTotal(hiding: hidden) == 1)
        let all = data.toggling(.init("B"), in: hidden)
        #expect(data.visibleTotal(hiding: all) == 0)
        #expect(data.toggling(.init("A"), in: all) == [.init("B")])
        let replacement = try chart([row("B", 1)])
        #expect(replacement.liveHiddenKeys(all).isEmpty)
        #expect(replacement.toggling(.init("B"), in: []).isEmpty)
    }

    private func row(_ name: String, _ value: Double) -> JSONValue { row(name, .number(value)) }

    private func row(_ name: String, _ value: JSONValue) -> JSONValue {
        .object([("name", .string(name)), ("amount", value), ("other", .number(777))])
    }

    private func chart(_ rows: [JSONValue], x: String = "", series: [String] = [],
                       percent: Bool = false) throws -> PolarChartData {
        let element = try Element(json: .object([
            ("type", .string("element")), ("typeName", .string("Chart")),
            ("props", .object([
                ("data", .array(rows)), ("x", .string(x)), ("series", .array(series.map(JSONValue.string))),
                ("format", .string(percent ? "percent" : "number")),
            ])),
            ("partial", .bool(false)), ("hasDynamicProps", .bool(false)),
        ]))
        return try #require(PolarChartData(element))
    }
}
