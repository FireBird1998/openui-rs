// shared/color.ts reads a hex colour or the three integer channels at the
// start of rgb()/rgba(). Alpha, and even the remainder of the RGB text, are
// irrelevant to its choice of ink.
import Foundation

func onColor(_ background: String) -> ThemeColor? {
    let text = JSWhitespace.trim(background)
    let rgb: ThemeColor
    if text.hasPrefix("#") {
        let digits = text.utf8.dropFirst()
        guard digits.count == 3 || digits.count == 6,
              digits.allSatisfy({ (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0) }),
              case let .color(color)? = CSSPaint.parse(text) else { return nil }
        rgb = color
    } else {
        var rest = text.unicodeScalars[...]
        func consume(_ word: String) -> Bool {
            guard rest.prefix(word.utf8.count).map(ASCII.lowercased).elementsEqual(word.unicodeScalars) else {
                return false
            }
            rest = rest.dropFirst(word.utf8.count)
            return true
        }
        guard consume("rgba(") || consume("rgb(") else { return nil }
        rest = rest.drop(while: JSWhitespace.isWhitespace)
        func channel() -> Double? {
            let digits = rest.prefix(while: ASCII.isDigit)
            guard !digits.isEmpty else { return nil }
            rest = rest[digits.endIndex...]
            return Double(String(digits)).map { $0 / 255 }
        }
        func separator() -> Bool {
            let separators = rest.prefix { $0 == "," || JSWhitespace.isWhitespace($0) }
            rest = rest[separators.endIndex...]
            return !separators.isEmpty
        }
        guard let r = channel(), separator(), let g = channel(), separator(), let b = channel() else { return nil }
        rgb = ThemeColor(red: r, green: g, blue: b)
    }
    func linear(_ v: Double) -> Double { v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
    let luminance = 0.2126 * linear(rgb.red) + 0.7152 * linear(rgb.green) + 0.0722 * linear(rgb.blue)
    return .hex(luminance > 0.36 ? 0x0d0d0d : 0xffffff)
}
