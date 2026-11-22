// Colours. Upstream's lib/functions/colors.ts, declared in its order, each
// with an implementation that gives what upstream's gives.
import OpenUINative

extension IntelligentUI {
    static let colorFunctions: [DefinedFunction] = [
        function("MixColor", "The hex color a fraction t of the way from one hex color to another, e.g. @MixColor(\"#ffffff\", \"#0285ff\", 0.5) for a heatmap or a tint", [
            .init("from", .string),
            .init("to", .string),
            .init("t", .number),
        ], returns: .string) { args in
            try .string(ColorFunctions.mixColor(from: args.stringArgument("from"), to: args.stringArgument("to"),
                                                t: args.numberArgument("t")))
        },
    ]
}

enum ColorFunctions {
    /// `MixColor`: each channel `Math.round(v + (w - v) * k)` with t clamped
    /// to [0, 1], written as two lowercase hex digits. The first colour that
    /// isn't hex throws, naming the argument as written.
    static func mixColor(from: String, to: String, t: Double) throws(FunctionError) -> String {
        let a = try channels(from)
        let b = try channels(to)
        // Math.min(1, Math.max(0, t)), where NaN stays NaN. JSON carries no
        // NaN, so it can't arrive today; upstream then writes "#NaNNaNNaN".
        let k = t.isNaN ? t : min(1, max(0, t))
        var out = "#"
        for (v, w) in zip(a, b) {
            let rounded = JavaScript.round(v + (w - v) * k)
            // Rounding is monotonic, so a finite result stays between v and
            // w, both in 0...255.
            guard let channel = Int(exactly: rounded) else {
                out += "NaN" // (NaN).toString(16).padStart(2, "0")
                continue
            }
            out += channel < 16 ? "0" + String(channel, radix: 16) : String(channel, radix: 16)
        }
        return out
    }

    /// `/^#?([\da-f]{3}|[\da-f]{6})$/i` on the trimmed colour: its three
    /// channels, with a 3-digit colour's digits doubled.
    private static func channels(_ color: String) throws(FunctionError) -> [Double] {
        var digits = Array(JavaScript.trim(color))
        if digits.first == UInt16(UInt8(ascii: "#")) { digits.removeFirst() }
        let values = digits.compactMap(hexDigit)
        guard values.count == digits.count, values.count == 3 || values.count == 6 else {
            throw FunctionError("not a hex color: \(color)")
        }
        var hex = (values.count == 3 ? values.flatMap { [$0, $0] } : values).makeIterator()
        var out: [Double] = []
        while let high = hex.next(), let low = hex.next() {
            out.append(Double(high * 16 + low))
        }
        return out
    }

    /// An ASCII hex digit's value, either case; nil for anything else.
    private static func hexDigit(_ unit: UInt16) -> Int? {
        switch unit {
        case 0x30...0x39: Int(unit - 0x30)
        case 0x41...0x46: Int(unit - 0x41 + 10)
        case 0x61...0x66: Int(unit - 0x61 + 10)
        default: nil
        }
    }
}
