// The model's prop values as the drawing reads them: upstream's
// shared/props.ts, rule for rule. A Color is a token or a raw CSS colour
// that passes upstream's guard; a Length is px or one of upstream's CSS
// lengths; a Radius is "sm" to "xl", "full" or px; gap and padding count 4px
// steps. Anything else is dropped, as upstream drops it. A px is a point.
import OpenUINative

/// The colour tokens a Color may name, by their names in upstream's
/// shared/tokens.ts.
enum ColorToken: String, CaseIterable, Sendable {
    case ink, muted, faint, canvas, surface, surfaceStrong, border
    case info, success, warning, danger, purple, pink
    case infoSoft, successSoft, warningSoft, dangerSoft, purpleSoft, pinkSoft
    case chart1, chart2, chart3, chart4, chart5, chart6
}

/// A Color prop that passed upstream's rules: a token, or raw CSS for the
/// page to read (which may still name no colour, as "banana" names none).
enum ColorValue: Equatable, Sendable {
    case token(ColorToken)
    case css(String)

    private static let status: [(String, ColorToken, ColorToken)] = [
        ("info", .info, .infoSoft), ("success", .success, .successSoft), ("warning", .warning, .warningSoft),
        ("danger", .danger, .dangerSoft), ("purple", .purple, .purpleSoft), ("pink", .pink, .pinkSoft),
    ]

    /// Upstream's FOREGROUND table.
    private static let foregroundTokens: [String: ColorToken] = {
        var table: [String: ColorToken] = ["ink": .ink, "muted": .muted, "faint": .faint, "canvas": .canvas,
                                           "surface": .surface]
        for (name, token, _) in status { table[name] = token }
        for (i, token) in [ColorToken.chart1, .chart2, .chart3, .chart4, .chart5, .chart6].enumerated() {
            table["chart\(i + 1)"] = token
        }
        return table
    }()

    /// Upstream's BACKGROUND table: the foreground's, plus surface-strong and
    /// border, with each status as its soft tint.
    private static let backgroundTokens: [String: ColorToken] = {
        var table = foregroundTokens
        table["surface-strong"] = .surfaceStrong
        table["border"] = .border
        for (name, _, soft) in status { table[name] = soft }
        return table
    }()

    /// The names every JavaScript object answers to: upstream's tables are
    /// plain objects, so `FOREGROUND["toString"]` is a function, not a
    /// colour, and the prop gives none.
    private static let objectPrototypeKeys: Set<String> = [
        "constructor", "__proto__", "__defineGetter__", "__defineSetter__", "__lookupGetter__",
        "__lookupSetter__", "hasOwnProperty", "isPrototypeOf", "propertyIsEnumerable", "toLocaleString",
        "toString", "valueOf",
    ]

    /// Text, icon and stroke colours (upstream's `fg`).
    static func foreground(_ value: JSONValue?) -> ColorValue? {
        lookup(value, in: foregroundTokens)
    }

    /// Backgrounds, where status tokens are soft tints (upstream's `bg`).
    static func background(_ value: JSONValue?) -> ColorValue? {
        lookup(value, in: backgroundTokens)
    }

    static func foreground(_ value: PropValue?) -> ColorValue? { foreground(value?.ruleScalar) }
    static func background(_ value: PropValue?) -> ColorValue? { background(value?.ruleScalar) }

    private static func lookup(_ value: JSONValue?, in table: [String: ColorToken]) -> ColorValue? {
        guard case let .string(text)? = value else { return nil }
        if let token = table[text] { return .token(token) }
        if objectPrototypeKeys.contains(text) { return nil }
        let trimmed = JSWhitespace.trim(text)
        guard !trimmed.isEmpty, CSSGuard.isSafe(text) else { return nil }
        return .css(trimmed)
    }
}

/// Upstream's `UNSAFE_CSS` guard: a model-written style value could
/// otherwise load a resource (`url(...)`) or break out of its declaration.
///
///     /(url|image|image-set|cross-fade|element|expression|paint|var|attr|env)\s*\(|@import|[\\;{}<>]/i
enum CSSGuard {
    private static let functions: [[Unicode.Scalar]] = [
        "url", "image", "image-set", "cross-fade", "element", "expression", "paint", "var", "attr", "env",
    ].map { Array($0.unicodeScalars) }
    private static let atImport = Array("@import".unicodeScalars)
    private static let refused: Set<Unicode.Scalar> = ["\\", ";", "{", "}", "<", ">"]

    static func isSafe(_ value: String) -> Bool {
        // ASCII letters folded, as a non-Unicode JavaScript regular
        // expression with `i` folds: never a non-ASCII letter.
        let text = value.unicodeScalars.map(ASCII.lowercased)
        for i in text.indices {
            if refused.contains(text[i]) { return false }
            if matches(atImport, in: text, at: i) { return false }
            for name in functions where matches(name, in: text, at: i) {
                var j = i + name.count
                while j < text.count, JSWhitespace.isWhitespace(text[j]) { j += 1 }
                if j < text.count, text[j] == "(" { return false }
            }
        }
        return true
    }

    private static func matches(_ word: [Unicode.Scalar], in text: [Unicode.Scalar], at i: Int) -> Bool {
        guard i + word.count <= text.count else { return false }
        for k in word.indices where text[i + k] != word[k] { return false }
        return true
    }
}

/// JavaScript's whitespace, as `String.prototype.trim` and a regular
/// expression's `\s` know it: WhiteSpace and LineTerminator.
enum JSWhitespace {
    static func isWhitespace(_ s: Unicode.Scalar) -> Bool {
        switch s.value {
        case 0x09...0x0D, 0x20, 0xA0, 0x1680, 0x2000...0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF:
            true
        default:
            false
        }
    }

    static func trim(_ text: String) -> String {
        let scalars = text.unicodeScalars
        guard let first = scalars.firstIndex(where: { !isWhitespace($0) }),
              let last = scalars.lastIndex(where: { !isWhitespace($0) }) else { return "" }
        return String(scalars[first...last])
    }
}

/// A Length prop (upstream's `size`): a number is points, negatives clamped
/// to 0; a string of digits is points; else one of upstream's units or
/// `auto`, written exactly so (`12PX` and `-12px` are dropped).
enum Length: Equatable, Sendable {
    case points(Double)
    case percent(Double)
    case em(Double)
    case rem(Double)
    case vh(Double)
    case vw(Double)
    case fr(Double)
    case auto

    init?(_ value: JSONValue?) {
        switch value {
        case let .number(n)?:
            guard n.isFinite else { return nil }
            self = .points(max(0, n))
        case let .string(text)?:
            let trimmed = JSWhitespace.trim(text)
            if trimmed == "auto" { self = .auto; return }
            guard let (number, unit) = Self.split(trimmed) else { return nil }
            switch unit {
            case "", "px": self = .points(number)
            case "%": self = .percent(number)
            case "em": self = .em(number)
            case "rem": self = .rem(number)
            case "vh": self = .vh(number)
            case "vw": self = .vw(number)
            case "fr": self = .fr(number)
            default: return nil
            }
        default:
            return nil
        }
    }

    init?(_ value: PropValue?) { self.init(value?.ruleScalar) }

    /// `^\d+(\.\d+)?` and the rest, with ASCII digits only.
    private static func split(_ text: String) -> (Double, String)? {
        let scalars = Array(text.unicodeScalars)
        var i = 0
        while i < scalars.count, ASCII.isDigit(scalars[i]) { i += 1 }
        guard i > 0 else { return nil }
        if i < scalars.count, scalars[i] == ".", i + 1 < scalars.count, ASCII.isDigit(scalars[i + 1]) {
            i += 1
            while i < scalars.count, ASCII.isDigit(scalars[i]) { i += 1 }
        }
        guard let number = Double(String(String.UnicodeScalarView(scalars[0..<i]))) else { return nil }
        return (finite(number), String(String.UnicodeScalarView(scalars[i...])))
    }

    /// A length too long for a Double is the largest one, as a browser
    /// clamps a huge length.
    private static func finite(_ n: Double) -> Double {
        n.isFinite ? n : .greatestFiniteMagnitude
    }
}

/// The theme's corner radii a Radius may name.
enum RadiusToken: String, CaseIterable, Sendable {
    case sm, md, lg, xl
}

/// A Radius prop (upstream's `radius`): "sm" to "xl" name the theme's radii,
/// "full" is 999 points, and a number is a Length's points. A string is only
/// ever a word: "12px" is dropped.
enum Radius: Equatable, Sendable {
    case token(RadiusToken)
    case points(Double)

    init?(_ value: JSONValue?) {
        switch value {
        case let .string(word)?:
            if word == "full" { self = .points(999); return }
            guard let token = RadiusToken(rawValue: word) else { return nil }
            self = .token(token)
        case .number?:
            guard case let .points(p)? = Length(value) else { return nil }
            self = .points(p)
        default:
            return nil
        }
    }

    init?(_ value: PropValue?) { self.init(value?.ruleScalar) }
}

/// Gap and padding (upstream's `space`): a number of 4-point steps,
/// negatives clamped to 0.
enum Spacing {
    static func points(_ value: JSONValue?) -> Double? {
        guard case let .number(n)? = value, n.isFinite else { return nil }
        let points = max(0, n) * 4
        // The browser drops the non-length "Infinitypx".
        return points.isFinite ? points : nil
    }

    static func points(_ value: PropValue?) -> Double? { points(value?.ruleScalar) }
}

extension PropValue {
    /// The string, number, boolean or null, or nil for anything else: what
    /// upstream's helpers see as a non-string, non-number value.
    var ruleScalar: JSONValue? {
        if case let .scalar(value) = self { value } else { nil }
    }
}
