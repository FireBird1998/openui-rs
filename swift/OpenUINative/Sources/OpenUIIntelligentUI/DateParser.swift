// String input to dates.ts has two paths: its own strict calendar-day test,
// then JavaScriptCore's ISO date-time and legacy English date grammars.
// A scanner keeps one token of lookahead. Even an arbitrarily long number,
// word or parenthesized comment takes one pass and constant scratch space.
import Foundation

enum DateParser {
    private static let limit = CivilDate.dayLimit * millisecondsPerDay

    /// TimeClip from ECMAScript. Check the bound before converting a host number to Int.
    static func clip(_ milliseconds: Double) -> Int? {
        guard abs(milliseconds) <= Double(limit) else { return nil }
        return Int(milliseconds)
    }

    static func milliseconds(_ text: String, in zone: TimeZone) -> Int? {
        guard let parsed = instant(text) else { return nil }
        let wall = parsed.milliseconds
        let offset: Int
        if let explicit = parsed.offsetSeconds {
            offset = explicit * 1000
        } else {
            guard abs(wall) <= limit + millisecondsPerDay else { return nil }
            let lookup: Int
            if abs(wall) > limit {
                // Bun looks up the offset in an equivalent civil year just outside
                // TimeClip. Only the offset comes from that year.
                let day = wall < 0 ? CivilDate.days(year: 2027, month: 4, date: 19)
                                   : CivilDate.days(year: 2008, month: 9, date: 13)
                lookup = day * millisecondsPerDay + floorMod(wall, millisecondsPerDay)
            } else {
                lookup = wall
            }
            // At gaps and overlaps Date takes the offset before the transition.
            // When yesterday's offset gives an inconsistent candidate, try the
            // offset at that instant. A gap has no consistent candidate.
            let before = zone.offsetMilliseconds(at: lookup - millisecondsPerDay)
            let candidate = zone.offsetMilliseconds(at: lookup - before)
            offset = zone.offsetMilliseconds(at: lookup - candidate) == candidate ? candidate : before
        }
        return clip(Double(wall - offset))
    }

    private struct Instant {
        let milliseconds: Int
        let offsetSeconds: Int? // nil means viewer-local wall time
    }

    static func plainDate(_ text: String) -> (Int, Int, Int)? {
        let trimmed = JavaScript.trim(text)
        guard trimmed.count == 10 else { return nil }
        var input = Scanner(trimmed)
        guard let y = input.number(4), input.skip(.minus), let m = input.number(2),
              input.skip(.minus), let d = input.number(2), input.peek == .end else { return nil }
        return (y, m, d)
    }

    private static func instant(_ text: String) -> Instant? {
        var input = Scanner(text[...].utf16)
        var fields = DayFields()
        // Consume the common ISO prefix. A non-ISO suffix falls back to
        // legacy parsing with the date fields already consumed. After T, a
        // malformed time is always invalid.
        if input.peek == .minus || input.peek == .plus {
            let sign = input.pop() == .minus ? -1 : 1
            // A leading sign is ignored by the legacy grammar. A six-digit
            // -000000 consumes the zero year too before that fallback.
            guard let year = input.number(6), year != 0 || sign > 0 else {
                return legacy(&input, fields: fields)
            }
            fields.parts.append(sign * year)
        } else if let year = input.number(4) {
            fields.parts.append(year)
        } else {
            return legacy(&input, fields: fields)
        }
        if input.skip(.minus) {
            guard let month = input.number(2, in: 1...12) else { return legacy(&input, fields: fields) }
            fields.parts.append(month)
            if input.skip(.minus) {
                guard let date = input.number(2, in: 1...31) else { return legacy(&input, fields: fields) }
                fields.parts.append(date)
            }
        }
        guard input.skip(.word("t")) else {
            if input.peek == .end { return fields.instant(iso: true, time: 0, offset: 0) }
            return legacy(&input, fields: fields)
        }
        guard let h = input.number(2, in: 0...24), input.skip(.colon),
              let m = input.number(2, in: 0...(h == 24 ? 0 : 59)) else { return nil }
        var sec = 0, ms = 0
        if input.skip(.colon) {
            guard let s = input.number(2, in: 0...(h == 24 ? 0 : 59)) else { return nil }
            sec = s
            if input.skip(.dot) {
                guard case let .digits(n) = input.pop(), h < 24 || n.value == 0 else { return nil }
                ms = n.milliseconds
            }
        }
        var offset: Int?
        if input.skip(.word("z")) {
            offset = 0
        } else if input.peek == .plus || input.peek == .minus {
            let sign = input.pop() == .minus ? -1 : 1
            let hours: Int, minutes: Int
            if let hhmm = input.number(4) {
                hours = hhmm / 100
                minutes = hhmm % 100
            } else {
                guard let hh = input.number(2), input.skip(.colon), let mm = input.number(2) else { return nil }
                hours = hh
                minutes = mm
            }
            guard hours < 24 && minutes < 60 else { return nil }
            offset = sign * (hours * 60 + minutes) * 60
        }
        guard input.peek == .end else { return nil }
        return fields.instant(iso: true, time: ((h * 60 + m) * 60 + sec) * 1000 + ms, offset: offset)
    }

    private struct DayFields {
        var parts: [Int] = [] // at most three
        var namedMonth: Int?

        func instant(iso: Bool, time: Int, offset: Int?) -> Instant? {
            guard let first = parts.first else { return nil }
            let second = parts.dropFirst().first, third = parts.dropFirst(2).first
            var year: Int
            let month: Int, date: Int
            if let namedMonth {
                month = namedMonth
                if (1...31).contains(first) {
                    date = first
                    year = second ?? 1
                } else {
                    year = first
                    date = second ?? 1
                }
            } else if iso || !(1...31).contains(first) {
                year = first
                month = second ?? 1
                date = third ?? 1
            } else {
                month = first
                date = second ?? 1
                year = third ?? 1
            }
            if !iso && (0...99).contains(year) { year += year < 50 ? 2000 : 1900 }
            // Even the largest legacy offset (Int32.max seconds, under 69
            // years) cannot bring a year outside this interval into TimeClip.
            guard (-272_000...276_000).contains(year), (1...12).contains(month), (1...31).contains(date) else { return nil }
            return Instant(milliseconds: CivilDate.days(year: year, month: month, date: date) * millisecondsPerDay + time,
                           offsetSeconds: offset)
        }
    }

    private static let monthNames = ["jan": 1, "feb": 2, "mar": 3, "apr": 4, "may": 5, "jun": 6,
                                     "jul": 7, "aug": 8, "sep": 9, "oct": 10, "nov": 11, "dec": 12]
    private static let zones = ["ut": 0, "utc": 0, "gmt": 0, "z": 0, "cdt": -5, "cst": -6, "edt": -4, "est": -5,
                                "mdt": -6, "mst": -7, "pdt": -7, "pst": -8]

    private struct LegacyZone {
        var sign = 1
        var hours: Int?
        var minutes: Int?

        var isUTC: Bool { hours == 0 && minutes == 0 }
        var needsMinute: Bool { hours != nil && minutes == nil }

        // Bun multiplies legacy GMT offsets in unsigned 32-bit seconds, then
        // checks the signed range after it has read the entire string.
        var seconds: Int? {
            guard let hours else { return nil }
            let value = UInt32(truncatingIfNeeded: hours * 3600 + (minutes ?? 0) * 60)
            return value <= Int32.max ? sign * Int(value) : nil
        }

        mutating func read(sign: Int, input: inout Scanner) -> Bool {
            self.sign = sign
            var number: Digits?
            if case let .digits(n) = input.peek {
                number = n
                _ = input.pop()
            }
            if input.skip(.colon) {
                hours = number?.value ?? 0
                minutes = nil
                return true
            }
            guard let number, number.width <= 4 else { return false }
            hours = number.width <= 2 ? number.value : number.value / 100
            minutes = number.width <= 2 ? 0 : number.value % 100
            return true
        }
    }

    private static func legacy(_ input: inout Scanner, fields initial: DayFields) -> Instant? {
        var fields = initial
        var time: [Int] = [] // hour, minute, second, millisecond
        var meridiem: Int?
        var zone = LegacyZone()
        var seenNumber = !fields.parts.isEmpty
        while input.peek != .end {
            let token = input.pop()
            switch token {
            case let .digits(digits):
                seenNumber = true
                let n = digits.value
                let expectingTime = ((1...2).contains(time.count) && n < 60) || (time.count == 3 && n < 1000)
                if input.skip(.colon) {
                    if input.skip(.colon) {
                        guard time.isEmpty else { return nil }
                        time = [n, 0]
                    } else {
                        guard time.count < 4 else { return nil }
                        time.append(n)
                    }
                } else if input.skip(.dot), expectingTime {
                    guard case let .digits(fraction) = input.pop() else { return nil }
                    time.append(n)
                    if time.count < 4 { time.append(fraction.milliseconds) }
                    while time.count < 4 { time.append(0) }
                } else if zone.needsMinute, n < 60 {
                    zone.minutes = n
                } else if expectingTime {
                    time.append(n)
                    while time.count < 4 { time.append(0) }
                    switch input.peek {
                    case .end, .space, .plus, .minus, .word("z"): break
                    default: return nil
                    }
                } else {
                    guard fields.parts.count < 3 else { return nil }
                    fields.parts.append(n)
                    _ = input.skip(.minus)
                }
            case let .word(word):
                if let month = monthNames[String(word.prefix(3))] {
                    fields.namedMonth = month
                    _ = input.skip(.minus)
                } else if !time.isEmpty && (word == "am" || word == "pm") {
                    meridiem = word == "pm" ? 12 : 0
                } else if seenNumber, let hours = zones[word] {
                    zone = LegacyZone(sign: hours < 0 ? -1 : 1, hours: abs(hours), minutes: 0)
                } else {
                    if seenNumber { return nil }
                    if case .digits = input.peek { return nil }
                }
            case .minus, .plus:
                guard zone.isUTC || !time.isEmpty else {
                    if seenNumber { return nil }
                    continue
                }
                let sign = token == .minus ? -1 : 1
                guard zone.read(sign: sign, input: &input) else { return nil }
            case .close where seenNumber: return nil
            default: break
            }
        }
        var hour = time.first ?? 0
        let minute = time.dropFirst().first ?? 0, second = time.dropFirst(2).first ?? 0
        let ms = time.dropFirst(3).first ?? 0
        if let meridiem {
            guard hour <= 12 else { return nil }
            hour = hour % 12 + meridiem
        }
        guard hour <= 24, minute < 60, second < 60, ms < 1000,
              hour < 24 || (minute == 0 && second == 0 && ms == 0) else { return nil }
        guard zone.hours == nil || zone.seconds != nil else { return nil }
        return fields.instant(iso: false, time: ((hour * 60 + minute) * 60 + second) * 1000 + ms, offset: zone.seconds)
    }
}

private struct Digits: Equatable {
    let width: Int
    let value: Int
    let milliseconds: Int
}

private enum Token: Equatable {
    case digits(Digits), word(String), plus, minus, dot, colon, close, space, other, end
}

private struct Scanner {
    private var rest: Substring.UTF16View
    private(set) var peek: Token = .end

    init(_ input: Substring.UTF16View) {
        rest = input
        peek = read()
    }

    mutating func pop() -> Token {
        let current = peek
        peek = read()
        return current
    }

    mutating func skip(_ token: Token) -> Bool {
        guard peek == token else { return false }
        _ = pop()
        return true
    }

    mutating func number(_ width: Int, in range: ClosedRange<Int> = 0...999_999_999) -> Int? {
        guard case let .digits(n) = peek, n.width == width, range.contains(n.value) else { return nil }
        _ = pop()
        return n.value
    }

    private func isSpace(_ c: UInt16) -> Bool {
        JavaScript.isWhitespace(c) && c != 0x0b && c != 0x2028 && c != 0x2029
    }

    private mutating func read() -> Token {
        guard let first = rest.first, first != 0 else { return .end }
        if (48...57).contains(first) {
            var width = 0, value = 0
            var significant = 0
            while let c = rest.first, (48...57).contains(c) {
                rest = rest.dropFirst()
                let digit = Int(c - 48)
                if significant < 9 { value = value * 10 + digit }
                if value > 0 { significant += 1 }
                width += 1
            }
            var ms = value
            if width < 3 { ms *= width == 1 ? 100 : 10 }
            for _ in 3..<max(3, min(width, 9)) { ms /= 10 }
            return .digits(Digits(width: width, value: value, milliseconds: ms))
        }
        if first >= 65 && !isSpace(first) {
            // Only three letters distinguish keywords. Only month names may
            // have trailing letters, so keep one extra to distinguish them.
            var word: [UInt8] = []
            while let c = rest.first, c >= 65 && !isSpace(c) {
                if word.count < 4 { word.append(c <= 127 ? UInt8(c) | 32 : 127) }
                rest = rest.dropFirst()
            }
            return .word(String(decoding: word, as: UTF8.self))
        }
        rest = rest.dropFirst()
        switch first {
        case 40:
            var depth = 1
            while depth > 0, let c = rest.first, c != 0 {
                rest = rest.dropFirst()
                if c == 40 { depth += 1 }
                if c == 41 { depth -= 1 }
            }
            return .other
        case 41: return .close
        case 43: return .plus
        case 45: return .minus
        case 46: return .dot
        case 58: return .colon
        default: return isSpace(first) ? .space : .other
        }
    }
}

extension TimeZone {
    func offsetMilliseconds(at milliseconds: Int) -> Int {
        secondsFromGMT(for: Date(timeIntervalSince1970: Double(milliseconds) / 1000)) * 1000
    }
}
