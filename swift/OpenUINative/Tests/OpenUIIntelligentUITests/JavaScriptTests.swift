// The JavaScript behaviour the functions share, against what Bun 1.4 prints.
// MixColor's recorded pairs reach only part of it (its rounding never sees a
// negative number), so the rest is pinned here.
@testable import OpenUIIntelligentUI
import SwiftTestSupport
import Testing

@Suite(NativeTestWatchdog()) struct JavaScriptTests {
    @Test(arguments: [
        (-0.0, "0"), (42, "42"), (-2.125e-5, "-0.00002125"), (1e-6, "0.000001"),
        (1e-7, "1e-7"), (1e20, "100000000000000000000"), (1e21, "1e+21"),
    ])
    func primitiveNumberText(value: Double, expected: String) {
        #expect(JavaScript.numberString(value) == expected)
    }

    /// `Math.round(x)` in Bun 1.4, with -0 kept apart from 0.
    @Test(arguments: [
        (2.5, 3.0), (-2.5, -2.0), (-0.4, -0.0), (-0.5, -0.0), (0.49999999999999994, 0.0), (-0.0, -0.0),
        (1.5, 2.0), (-1.5, -1.0), (4503599627370495.5, 4503599627370496.0),
        (-4503599627370495.5, -4503599627370495.0), (1e300, 1e300), (-3.7, -4.0),
    ])
    func roundIsMathRound(x: Double, expected: Double) {
        let rounded = JavaScript.round(x)
        #expect(rounded == expected && rounded.sign == expected.sign)
    }

    @Test func trimRemovesOnlyJavaScriptsWhitespace() {
        let kept = "\u{85}a\u{180E}\u{200B}"
        #expect(Array(JavaScript.trim(" \t\u{2028}\u{3000}" + kept + "\u{FEFF}\n")) == Array(kept.utf16))
        #expect(Array(JavaScript.trim(" \u{A0} ")).isEmpty)
        #expect(Array(JavaScript.trim("")).isEmpty)
    }
}
