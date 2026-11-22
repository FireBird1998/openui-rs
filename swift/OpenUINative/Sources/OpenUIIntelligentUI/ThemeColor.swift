// A CSS colour, as a browser would paint it. Upstream passes a model's raw
// colour string to the page's CSS (shared/props.ts), and its theme writes
// colours as CSS (oklch(), hex, color-mix()), so the native side reads the
// same text: named colours, transparent and currentColor, hex, rgb(), hsl(),
// hwb(), lab(), lch(), oklab(), oklch() and color-mix() in srgb,
// srgb-linear, oklab or oklch. Anything else a browser knows (color(),
// relative colours, system colours, light-dark(), calc(), other mixing
// spaces) is not read: it is no colour, as an invalid value is in a browser.
//
// Conversions follow CSS Color 4: Björn Ottosson's OKLab matrices (the ones
// upstream's react-components uses), and CIE Lab and LCH in D50 adapted to
// D65 with Bradford. Colours keep their extended range (not clamped to sRGB),
// as a browser paints oklch() in the display's gamut.
import Foundation
import SwiftUI

/// A colour as extended-range, gamma-encoded sRGB, with its alpha.
public struct ThemeColor: Sendable, Equatable {
    public var red: Double
    public var green: Double
    public var blue: Double
    /// From 0 (transparent) to 1.
    public var alpha: Double

    public init(red: Double, green: Double, blue: Double, alpha: Double = 1) {
        self.red = red
        self.green = green
        self.blue = blue
        self.alpha = alpha
    }

    var isFinite: Bool { red.isFinite && green.isFinite && blue.isFinite && alpha.isFinite }

    /// The colour for SwiftUI, in extended sRGB.
    public var color: Color { Color(.sRGB, red: red, green: green, blue: blue, opacity: alpha) }

    /// `oklch(l c h / alpha)`.
    static func oklch(_ l: Double, _ c: Double, _ h: Double, alpha: Double = 1) -> ThemeColor {
        let radians = ColorSpaces.normalizedHue(h) * .pi / 180
        return ColorSpaces.fromOKLab(l, c * cos(radians), c * sin(radians), alpha: alpha)
    }

    /// `#rrggbb`.
    static func hex(_ value: UInt32) -> ThemeColor {
        ThemeColor(red: Double((value >> 16) & 0xff) / 255, green: Double((value >> 8) & 0xff) / 255,
                   blue: Double(value & 0xff) / 255)
    }

    /// `color-mix(in srgb, self <fraction * 100>%, transparent)`: the same
    /// colour, its alpha scaled.
    func mixedWithTransparent(_ fraction: Double) -> ThemeColor {
        var mixed = self
        mixed.alpha = alpha * fraction
        return mixed
    }

    /// `color-mix(in oklch, self <fraction * 100>%, other)`.
    func mixed(inOKLCH fraction: Double, with other: ThemeColor) -> ThemeColor {
        ColorMix.mix(.oklch, .shorter, self, fraction, other, 1 - fraction)
    }
}

/// What a CSS colour value paints: a colour, or the text colour it sits in.
public enum CSSPaint: Sendable, Equatable {
    case color(ThemeColor)
    /// `currentColor`: the text colour where it is used.
    case currentColor
}

extension CSSPaint {
    /// The paint a CSS colour value names, or nil when it names none this
    /// reader knows. Never fails on any text: nesting is limited and every
    /// loop ends with its input.
    static func parse(_ text: String) -> CSSPaint? {
        var parser = CSSColorParser(text)
        return parser.parseWhole()
    }
}

/// ASCII character tests shared by the colour reader and the value rules.
enum ASCII {
    static func isDigit(_ s: Unicode.Scalar) -> Bool { s.value >= 0x30 && s.value <= 0x39 }

    /// An ASCII capital lowered; anything else as it is.
    static func lowercased(_ s: Unicode.Scalar) -> Unicode.Scalar {
        s.value >= 0x41 && s.value <= 0x5A ? Unicode.Scalar(s.value + 32) ?? s : s
    }
}

// MARK: Colour spaces

enum ColorSpaces {
    static func toLinear(_ c: Double) -> Double {
        let a = abs(c)
        let v = a <= 0.04045 ? a / 12.92 : pow((a + 0.055) / 1.055, 2.4)
        return c < 0 ? -v : v
    }

    static func fromLinear(_ c: Double) -> Double {
        let a = abs(c)
        let v = a <= 0.0031308 ? a * 12.92 : 1.055 * pow(a, 1 / 2.4) - 0.055
        return c < 0 ? -v : v
    }

    static func fromLinearSRGB(_ r: Double, _ g: Double, _ b: Double, alpha: Double) -> ThemeColor {
        ThemeColor(red: fromLinear(r), green: fromLinear(g), blue: fromLinear(b), alpha: alpha)
    }

    static func linear(_ c: ThemeColor) -> (Double, Double, Double) {
        (toLinear(c.red), toLinear(c.green), toLinear(c.blue))
    }

    static func fromOKLab(_ l: Double, _ a: Double, _ b: Double, alpha: Double) -> ThemeColor {
        let l_ = l + 0.3963377774 * a + 0.2158037573 * b
        let m_ = l - 0.1055613458 * a - 0.0638541728 * b
        let s_ = l - 0.0894841775 * a - 1.2914855480 * b
        let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_)
        return fromLinearSRGB(
            4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
            -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
            -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
            alpha: alpha
        )
    }

    static func toOKLab(_ c: ThemeColor) -> (l: Double, a: Double, b: Double) {
        let (r, g, b) = linear(c)
        let l = cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b)
        let m = cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b)
        let s = cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b)
        return (0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
                1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
                0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s)
    }

    /// CIE Lab (D50) to sRGB, through XYZ adapted to D65 with Bradford.
    static func fromLab(_ lightness: Double, _ a: Double, _ b: Double, alpha: Double) -> ThemeColor {
        let kappa = 24389.0 / 27, epsilon = 216.0 / 24389
        let f1 = (lightness + 16) / 116
        let f0 = a / 500 + f1
        let f2 = f1 - b / 200
        let x = f0 * f0 * f0 > epsilon ? f0 * f0 * f0 : (116 * f0 - 16) / kappa
        let y = lightness > kappa * epsilon ? f1 * f1 * f1 : lightness / kappa
        let z = f2 * f2 * f2 > epsilon ? f2 * f2 * f2 : (116 * f2 - 16) / kappa
        let (X, Y, Z) = (x * 0.3457 / 0.3585, y, z * (1 - 0.3457 - 0.3585) / 0.3585)
        let X65 = 0.955473421488075 * X - 0.02309845494876471 * Y + 0.06325924320057072 * Z
        let Y65 = -0.0283697093338637 * X + 1.0099953980813041 * Y + 0.021041441191917323 * Z
        let Z65 = 0.012314014864481998 * X - 0.020507649298898964 * Y + 1.330365926242124 * Z
        return fromLinearSRGB(
            3.2409699419045226 * X65 - 1.537383177570094 * Y65 - 0.4986107602930034 * Z65,
            -0.9692436362808796 * X65 + 1.8759675015077202 * Y65 + 0.04155505740717559 * Z65,
            0.05563007969699366 * X65 - 0.20397695888897652 * Y65 + 1.0569715142428786 * Z65,
            alpha: alpha
        )
    }

    static func fromHSL(_ hue: Double, _ saturation: Double, _ lightness: Double, alpha: Double) -> ThemeColor {
        let h = normalizedHue(hue)
        func f(_ n: Double) -> Double {
            let k = (n + h / 30).truncatingRemainder(dividingBy: 12)
            let a = saturation * min(lightness, 1 - lightness)
            return lightness - a * max(-1, min(k - 3, 9 - k, 1))
        }
        return ThemeColor(red: f(0), green: f(8), blue: f(4), alpha: alpha)
    }

    static func fromHWB(_ hue: Double, _ white: Double, _ black: Double, alpha: Double) -> ThemeColor {
        if white + black >= 1 {
            let gray = white / (white + black)
            return ThemeColor(red: gray, green: gray, blue: gray, alpha: alpha)
        }
        let pure = fromHSL(hue, 1, 0.5, alpha: alpha)
        func f(_ c: Double) -> Double { c * (1 - white - black) + white }
        return ThemeColor(red: f(pure.red), green: f(pure.green), blue: f(pure.blue), alpha: alpha)
    }

    /// A hue in [0, 360). Not finite gives 0.
    static func normalizedHue(_ h: Double) -> Double {
        guard h.isFinite else { return 0 }
        let r = h.truncatingRemainder(dividingBy: 360)
        return r < 0 ? r + 360 : r
    }
}

// MARK: color-mix()

enum ColorMix {
    enum Space { case srgb, srgbLinear, oklab, oklch }
    enum HueMethod { case shorter, longer, increasing, decreasing }

    /// Chroma below this has no hue (CSS Color 4: the hue is powerless).
    static let achromatic = 1e-6

    /// Mixes two colours with their fractions, which sum to 1; premultiplied
    /// by alpha, as CSS Color 5 mixes.
    static func mix(_ space: Space, _ method: HueMethod, _ c1: ThemeColor, _ p1: Double,
                    _ c2: ThemeColor, _ p2: Double) -> ThemeColor {
        let alpha = c1.alpha * p1 + c2.alpha * p2
        func blend(_ x1: Double, _ x2: Double) -> Double {
            let premultiplied = x1 * c1.alpha * p1 + x2 * c2.alpha * p2
            return alpha == 0 ? x1 * p1 + x2 * p2 : premultiplied / alpha
        }
        switch space {
        case .srgb:
            return ThemeColor(red: blend(c1.red, c2.red), green: blend(c1.green, c2.green),
                              blue: blend(c1.blue, c2.blue), alpha: alpha)
        case .srgbLinear:
            let (r1, g1, b1) = ColorSpaces.linear(c1), (r2, g2, b2) = ColorSpaces.linear(c2)
            return ColorSpaces.fromLinearSRGB(blend(r1, r2), blend(g1, g2), blend(b1, b2), alpha: alpha)
        case .oklab:
            let a = ColorSpaces.toOKLab(c1), b = ColorSpaces.toOKLab(c2)
            return ColorSpaces.fromOKLab(blend(a.l, b.l), blend(a.a, b.a), blend(a.b, b.b), alpha: alpha)
        case .oklch:
            let a = ColorSpaces.toOKLab(c1), b = ColorSpaces.toOKLab(c2)
            let chroma1 = (a.a * a.a + a.b * a.b).squareRoot(), chroma2 = (b.a * b.a + b.b * b.b).squareRoot()
            // A powerless hue is missing, and takes the other colour's.
            var h1: Double? = chroma1 < achromatic ? nil : ColorSpaces.normalizedHue(atan2(a.b, a.a) * 180 / .pi)
            var h2: Double? = chroma2 < achromatic ? nil : ColorSpaces.normalizedHue(atan2(b.b, b.a) * 180 / .pi)
            if h1 == nil { h1 = h2 }
            if h2 == nil { h2 = h1 }
            let l = blend(a.l, b.l), c = blend(chroma1, chroma2)
            guard var x = h1, var y = h2 else { return ColorSpaces.fromOKLab(l, 0, 0, alpha: alpha) }
            switch method {
            case .shorter:
                if y - x > 180 { x += 360 } else if y - x < -180 { y += 360 }
            case .longer:
                if 0 < y - x && y - x < 180 { x += 360 } else if -180 < y - x && y - x <= 0 { y += 360 }
            case .increasing:
                if y < x { y += 360 }
            case .decreasing:
                if x < y { x += 360 }
            }
            let h = (x * p1 + y * p2) * .pi / 180
            return ColorSpaces.fromOKLab(l, c * cos(h), c * sin(h), alpha: alpha)
        }
    }
}

// MARK: The reader

/// Reads one CSS colour value. Works on Unicode scalars; CSS keywords and
/// function names are ASCII and case-insensitive.
struct CSSColorParser {
    /// How deep color-mix() may nest.
    static let maxDepth = 16

    private enum Token: Equatable {
        case number(Double)
        case percentage(Double)
        case dimension(Double, String)
        case ident(String)
        case hash(String)
        case comma
        case slash
        /// A function: its lowercased name and its arguments' tokens.
        case function(String, [Token])
    }

    private let scalars: [Unicode.Scalar]
    private var at = 0
    private var depth = 0

    init(_ text: String) {
        scalars = Array(text.unicodeScalars)
    }

    mutating func parseWhole() -> CSSPaint? {
        guard let tokens = tokenize(), tokens.count == 1 else { return nil }
        let result = paint(tokens[0])
        // Huge channels can overflow a conversion: such a value is no colour.
        if case let .color(c)? = result, !c.isFinite { return nil }
        return result
    }

    // MARK: Tokens

    private static func isSpace(_ s: Unicode.Scalar) -> Bool {
        s == " " || s == "\t" || s == "\n" || s == "\r" || s == "\u{0C}"
    }

    private static func isDigit(_ s: Unicode.Scalar) -> Bool { ASCII.isDigit(s) }

    private static func isNameStart(_ s: Unicode.Scalar) -> Bool {
        (s.value >= 0x41 && s.value <= 0x5A) || (s.value >= 0x61 && s.value <= 0x7A) || s == "_" || s.value >= 0x80
    }

    private static func isName(_ s: Unicode.Scalar) -> Bool { isNameStart(s) || isDigit(s) || s == "-" }

    private func peek(_ offset: Int = 0) -> Unicode.Scalar? {
        at + offset < scalars.count ? scalars[at + offset] : nil
    }

    private static func lowercasedASCII(_ text: String) -> String {
        String(String.UnicodeScalarView(text.unicodeScalars.map(ASCII.lowercased)))
    }

    /// The tokens up to the end, or up to the `)` closing the current
    /// function, which is consumed. Nil on anything this reader doesn't know.
    private mutating func tokenize() -> [Token]? {
        var tokens: [Token] = []
        while let s = peek() {
            if Self.isSpace(s) { at += 1; continue }
            if s == ")" {
                guard depth > 0 else { return nil }
                at += 1
                return tokens
            }
            if s == "," { at += 1; tokens.append(.comma); continue }
            if s == "/" { at += 1; tokens.append(.slash); continue }
            if s == "#" {
                at += 1
                let name = readName()
                guard !name.isEmpty else { return nil }
                tokens.append(.hash(name))
                continue
            }
            if startsNumber() {
                guard let token = readNumeric() else { return nil }
                tokens.append(token)
                continue
            }
            if startsIdent() {
                let name = Self.lowercasedASCII(readName())
                if peek() == "(" {
                    at += 1
                    guard depth < Self.maxDepth else { return nil }
                    depth += 1
                    guard let args = tokenize() else { return nil }
                    depth -= 1
                    tokens.append(.function(name, args))
                } else {
                    tokens.append(.ident(name))
                }
                continue
            }
            return nil
        }
        // The end closes any open function, as in CSS.
        return tokens
    }

    private func startsIdent() -> Bool {
        guard let s = peek() else { return false }
        if Self.isNameStart(s) { return true }
        if s == "-", let next = peek(1) { return Self.isNameStart(next) || next == "-" }
        return false
    }

    private func startsNumber() -> Bool {
        guard let s = peek() else { return false }
        if Self.isDigit(s) { return true }
        if s == ".", let n = peek(1) { return Self.isDigit(n) }
        if s == "+" || s == "-", let n = peek(1) {
            if Self.isDigit(n) { return true }
            if n == ".", let d = peek(2) { return Self.isDigit(d) }
        }
        return false
    }

    private mutating func readName() -> String {
        var out = String.UnicodeScalarView()
        while let s = peek(), Self.isName(s) {
            out.append(s)
            at += 1
        }
        return String(out)
    }

    private mutating func readNumeric() -> Token? {
        var text = String.UnicodeScalarView()
        if let s = peek(), s == "+" || s == "-" { text.append(s); at += 1 }
        while let s = peek(), Self.isDigit(s) { text.append(s); at += 1 }
        if peek() == ".", let d = peek(1), Self.isDigit(d) {
            text.append("."); at += 1
            while let s = peek(), Self.isDigit(s) { text.append(s); at += 1 }
        }
        if let e = peek(), e == "e" || e == "E" {
            var offset = 1
            if let sign = peek(1), sign == "+" || sign == "-" { offset = 2 }
            if let d = peek(offset), Self.isDigit(d) {
                for i in 0..<offset { if let s = peek(i) { text.append(s) } }
                at += offset
                while let s = peek(), Self.isDigit(s) { text.append(s); at += 1 }
            }
        }
        guard let value = Double(String(text)), value.isFinite else { return nil }
        if peek() == "%" { at += 1; return .percentage(value) }
        if startsIdent() { return .dimension(value, Self.lowercasedASCII(readName())) }
        return .number(value)
    }

    // MARK: Values

    private func paint(_ token: Token) -> CSSPaint? {
        switch token {
        case let .hash(digits): return Self.hex(digits).map(CSSPaint.color)
        case let .ident(name):
            if name == "currentcolor" { return .currentColor }
            if name == "transparent" { return .color(ThemeColor(red: 0, green: 0, blue: 0, alpha: 0)) }
            return namedColors[name].map { .color(.hex($0)) }
        case let .function(name, args):
            if name == "color-mix" { return mix(args).map(CSSPaint.color) }
            return colorFunction(name, args).map(CSSPaint.color)
        default: return nil
        }
    }

    private static func hex(_ digits: String) -> ThemeColor? {
        var values: [Double] = []
        for s in digits.unicodeScalars {
            guard let v = Int(String(s), radix: 16) else { return nil }
            values.append(Double(v))
        }
        switch values.count {
        case 3, 4:
            let c = values.map { ($0 * 16 + $0) / 255 }
            return ThemeColor(red: c[0], green: c[1], blue: c[2], alpha: c.count == 4 ? c[3] : 1)
        case 6, 8:
            let c = stride(from: 0, to: values.count, by: 2).map { (values[$0] * 16 + values[$0 + 1]) / 255 }
            return ThemeColor(red: c[0], green: c[1], blue: c[2], alpha: c.count == 4 ? c[3] : 1)
        default: return nil
        }
    }

    private enum Arg { case number(Double), percentage(Double), angle(Double), none }

    /// A colour function's arguments: three channels and the alpha, with
    /// whether they were written the legacy way (commas).
    private static func channels(_ tokens: [Token], legacyAllowed: Bool) -> (args: [Arg], alpha: Arg?, legacy: Bool)? {
        func arg(_ t: Token) -> Arg? {
            switch t {
            case let .number(v): return .number(v)
            case let .percentage(v): return .percentage(v)
            case let .dimension(v, unit):
                switch unit {
                case "deg": return .angle(v)
                case "rad": return .angle(v * 180 / .pi)
                case "grad": return .angle(v * 0.9)
                case "turn": return .angle(v * 360)
                default: return nil
                }
            case .ident("none"): return Arg.none
            default: return nil
            }
        }
        if tokens.contains(.comma) {
            guard legacyAllowed, tokens.count == 5 || tokens.count == 7 else { return nil }
            var args: [Arg] = []
            for (i, t) in tokens.enumerated() {
                if i % 2 == 1 { guard t == .comma else { return nil }; continue }
                guard let a = arg(t) else { return nil }
                if case .none = a { return nil }
                args.append(a)
            }
            return (Array(args.prefix(3)), args.count == 4 ? args[3] : nil, true)
        }
        var parts = tokens
        var alpha: Arg?
        if let slash = parts.firstIndex(of: .slash) {
            guard slash == parts.count - 2, let a = arg(parts[parts.count - 1]) else { return nil }
            alpha = a
            parts = Array(parts.prefix(slash))
        }
        guard parts.count == 3 else { return nil }
        var args: [Arg] = []
        for t in parts {
            guard let a = arg(t) else { return nil }
            args.append(a)
        }
        return (args, alpha, false)
    }

    private static func alphaValue(_ a: Arg?) -> Double? {
        switch a {
        case nil: return 1
        case let .number(v)?: return min(1, max(0, v))
        case let .percentage(v)?: return min(1, max(0, v / 100))
        case .none?: return 0
        case .angle?: return nil
        }
    }

    /// A number or percentage, a percentage scaled so 100% is `full`.
    private static func amount(_ a: Arg, full: Double) -> Double? {
        switch a {
        case let .number(v): return v
        case let .percentage(v): return v / 100 * full
        case .none: return 0
        case .angle: return nil
        }
    }

    private static func hue(_ a: Arg) -> Double? {
        switch a {
        case let .number(v), let .angle(v): return v
        case .none: return 0
        case .percentage: return nil
        }
    }

    private func colorFunction(_ name: String, _ tokens: [Token]) -> ThemeColor? {
        let legacyAllowed = ["rgb", "rgba", "hsl", "hsla"].contains(name)
        guard let (args, alphaArg, legacy) = Self.channels(tokens, legacyAllowed: legacyAllowed),
              let alpha = Self.alphaValue(alphaArg) else { return nil }
        let (a0, a1, a2) = (args[0], args[1], args[2])
        switch name {
        case "rgb", "rgba":
            if legacy {
                // All numbers or all percentages.
                let percents = args.filter { if case .percentage = $0 { true } else { false } }.count
                guard percents == 0 || percents == 3 else { return nil }
            }
            var c: [Double] = []
            for a in args {
                guard let v = Self.amount(a, full: 255) else { return nil }
                c.append(min(1, max(0, v / 255)))
            }
            return ThemeColor(red: c[0], green: c[1], blue: c[2], alpha: alpha)
        case "hsl", "hsla":
            if legacy {
                guard case .percentage = a1, case .percentage = a2 else { return nil }
            }
            guard let h = Self.hue(a0), let s = Self.amount(a1, full: 100), let l = Self.amount(a2, full: 100)
            else { return nil }
            return ColorSpaces.fromHSL(h, min(1, max(0, s / 100)), min(1, max(0, l / 100)), alpha: alpha)
        case "hwb":
            guard let h = Self.hue(a0), let w = Self.amount(a1, full: 100), let b = Self.amount(a2, full: 100)
            else { return nil }
            return ColorSpaces.fromHWB(h, min(1, max(0, w / 100)), min(1, max(0, b / 100)), alpha: alpha)
        case "lab":
            guard let l = Self.amount(a0, full: 100), let a = Self.amount(a1, full: 125),
                  let b = Self.amount(a2, full: 125) else { return nil }
            return ColorSpaces.fromLab(min(100, max(0, l)), a, b, alpha: alpha)
        case "lch":
            guard let l = Self.amount(a0, full: 100), let c = Self.amount(a1, full: 150), let h = Self.hue(a2)
            else { return nil }
            let r = ColorSpaces.normalizedHue(h) * .pi / 180, chroma = max(0, c)
            return ColorSpaces.fromLab(min(100, max(0, l)), chroma * cos(r), chroma * sin(r), alpha: alpha)
        case "oklab":
            guard let l = Self.amount(a0, full: 1), let a = Self.amount(a1, full: 0.4),
                  let b = Self.amount(a2, full: 0.4) else { return nil }
            return ColorSpaces.fromOKLab(min(1, max(0, l)), a, b, alpha: alpha)
        case "oklch":
            guard let l = Self.amount(a0, full: 1), let c = Self.amount(a1, full: 0.4), let h = Self.hue(a2)
            else { return nil }
            return .oklch(min(1, max(0, l)), max(0, c), h, alpha: alpha)
        default:
            return nil
        }
    }

    /// `color-mix(in <space> [<method> hue], <color> [<p>%], <color> [<p>%])`.
    private func mix(_ tokens: [Token]) -> ThemeColor? {
        let parts = tokens.split(separator: .comma, omittingEmptySubsequences: false).map(Array.init)
        guard parts.count == 3, parts[0].first == .ident("in") else { return nil }
        let spaceWords = parts[0].dropFirst()
        let space: ColorMix.Space
        switch spaceWords.first {
        case .ident("srgb")?: space = .srgb
        case .ident("srgb-linear")?: space = .srgbLinear
        case .ident("oklab")?: space = .oklab
        case .ident("oklch")?: space = .oklch
        default: return nil
        }
        var method = ColorMix.HueMethod.shorter
        let rest = Array(spaceWords.dropFirst())
        if !rest.isEmpty {
            guard space == .oklch, rest.count == 2, rest[1] == .ident("hue") else { return nil }
            switch rest[0] {
            case .ident("shorter"): method = .shorter
            case .ident("longer"): method = .longer
            case .ident("increasing"): method = .increasing
            case .ident("decreasing"): method = .decreasing
            default: return nil
            }
        }
        func side(_ part: [Token]) -> (ThemeColor, Double?)? {
            var percent: Double?
            var color: [Token] = []
            for t in part {
                if case let .percentage(p) = t {
                    guard percent == nil, p >= 0, p <= 100 else { return nil }
                    percent = p
                } else {
                    color.append(t)
                }
            }
            guard color.count == 1, case let .color(c)? = paint(color[0]) else { return nil }
            return (c, percent)
        }
        guard let (c1, q1) = side(parts[1]), let (c2, q2) = side(parts[2]) else { return nil }
        var p1 = q1 ?? (q2.map { 100 - $0 } ?? 50)
        var p2 = q2 ?? (100 - p1)
        let sum = p1 + p2
        guard sum > 0 else { return nil }
        let multiplier = sum < 100 ? sum / 100 : 1
        p1 /= sum
        p2 /= sum
        var mixed = ColorMix.mix(space, method, c1, p1, c2, p2)
        mixed.alpha *= multiplier
        return mixed
    }
}

/// CSS Color 4's named colours.
private let namedColors: [String: UInt32] = [
    "aliceblue": 0xf0f8ff,
    "antiquewhite": 0xfaebd7,
    "aqua": 0x00ffff,
    "aquamarine": 0x7fffd4,
    "azure": 0xf0ffff,
    "beige": 0xf5f5dc,
    "bisque": 0xffe4c4,
    "black": 0x000000,
    "blanchedalmond": 0xffebcd,
    "blue": 0x0000ff,
    "blueviolet": 0x8a2be2,
    "brown": 0xa52a2a,
    "burlywood": 0xdeb887,
    "cadetblue": 0x5f9ea0,
    "chartreuse": 0x7fff00,
    "chocolate": 0xd2691e,
    "coral": 0xff7f50,
    "cornflowerblue": 0x6495ed,
    "cornsilk": 0xfff8dc,
    "crimson": 0xdc143c,
    "cyan": 0x00ffff,
    "darkblue": 0x00008b,
    "darkcyan": 0x008b8b,
    "darkgoldenrod": 0xb8860b,
    "darkgray": 0xa9a9a9,
    "darkgreen": 0x006400,
    "darkgrey": 0xa9a9a9,
    "darkkhaki": 0xbdb76b,
    "darkmagenta": 0x8b008b,
    "darkolivegreen": 0x556b2f,
    "darkorange": 0xff8c00,
    "darkorchid": 0x9932cc,
    "darkred": 0x8b0000,
    "darksalmon": 0xe9967a,
    "darkseagreen": 0x8fbc8f,
    "darkslateblue": 0x483d8b,
    "darkslategray": 0x2f4f4f,
    "darkslategrey": 0x2f4f4f,
    "darkturquoise": 0x00ced1,
    "darkviolet": 0x9400d3,
    "deeppink": 0xff1493,
    "deepskyblue": 0x00bfff,
    "dimgray": 0x696969,
    "dimgrey": 0x696969,
    "dodgerblue": 0x1e90ff,
    "firebrick": 0xb22222,
    "floralwhite": 0xfffaf0,
    "forestgreen": 0x228b22,
    "fuchsia": 0xff00ff,
    "gainsboro": 0xdcdcdc,
    "ghostwhite": 0xf8f8ff,
    "gold": 0xffd700,
    "goldenrod": 0xdaa520,
    "gray": 0x808080,
    "green": 0x008000,
    "greenyellow": 0xadff2f,
    "grey": 0x808080,
    "honeydew": 0xf0fff0,
    "hotpink": 0xff69b4,
    "indianred": 0xcd5c5c,
    "indigo": 0x4b0082,
    "ivory": 0xfffff0,
    "khaki": 0xf0e68c,
    "lavender": 0xe6e6fa,
    "lavenderblush": 0xfff0f5,
    "lawngreen": 0x7cfc00,
    "lemonchiffon": 0xfffacd,
    "lightblue": 0xadd8e6,
    "lightcoral": 0xf08080,
    "lightcyan": 0xe0ffff,
    "lightgoldenrodyellow": 0xfafad2,
    "lightgray": 0xd3d3d3,
    "lightgreen": 0x90ee90,
    "lightgrey": 0xd3d3d3,
    "lightpink": 0xffb6c1,
    "lightsalmon": 0xffa07a,
    "lightseagreen": 0x20b2aa,
    "lightskyblue": 0x87cefa,
    "lightslategray": 0x778899,
    "lightslategrey": 0x778899,
    "lightsteelblue": 0xb0c4de,
    "lightyellow": 0xffffe0,
    "lime": 0x00ff00,
    "limegreen": 0x32cd32,
    "linen": 0xfaf0e6,
    "magenta": 0xff00ff,
    "maroon": 0x800000,
    "mediumaquamarine": 0x66cdaa,
    "mediumblue": 0x0000cd,
    "mediumorchid": 0xba55d3,
    "mediumpurple": 0x9370db,
    "mediumseagreen": 0x3cb371,
    "mediumslateblue": 0x7b68ee,
    "mediumspringgreen": 0x00fa9a,
    "mediumturquoise": 0x48d1cc,
    "mediumvioletred": 0xc71585,
    "midnightblue": 0x191970,
    "mintcream": 0xf5fffa,
    "mistyrose": 0xffe4e1,
    "moccasin": 0xffe4b5,
    "navajowhite": 0xffdead,
    "navy": 0x000080,
    "oldlace": 0xfdf5e6,
    "olive": 0x808000,
    "olivedrab": 0x6b8e23,
    "orange": 0xffa500,
    "orangered": 0xff4500,
    "orchid": 0xda70d6,
    "palegoldenrod": 0xeee8aa,
    "palegreen": 0x98fb98,
    "paleturquoise": 0xafeeee,
    "palevioletred": 0xdb7093,
    "papayawhip": 0xffefd5,
    "peachpuff": 0xffdab9,
    "peru": 0xcd853f,
    "pink": 0xffc0cb,
    "plum": 0xdda0dd,
    "powderblue": 0xb0e0e6,
    "purple": 0x800080,
    "rebeccapurple": 0x663399,
    "red": 0xff0000,
    "rosybrown": 0xbc8f8f,
    "royalblue": 0x4169e1,
    "saddlebrown": 0x8b4513,
    "salmon": 0xfa8072,
    "sandybrown": 0xf4a460,
    "seagreen": 0x2e8b57,
    "seashell": 0xfff5ee,
    "sienna": 0xa0522d,
    "silver": 0xc0c0c0,
    "skyblue": 0x87ceeb,
    "slateblue": 0x6a5acd,
    "slategray": 0x708090,
    "slategrey": 0x708090,
    "snow": 0xfffafa,
    "springgreen": 0x00ff7f,
    "steelblue": 0x4682b4,
    "tan": 0xd2b48c,
    "teal": 0x008080,
    "thistle": 0xd8bfd8,
    "tomato": 0xff6347,
    "turquoise": 0x40e0d0,
    "violet": 0xee82ee,
    "wheat": 0xf5deb3,
    "white": 0xffffff,
    "whitesmoke": 0xf5f5f5,
    "yellow": 0xffff00,
    "yellowgreen": 0x9acd32,
]
