// Proleptic Gregorian dates, including astronomical year zero. Foundation's
// Gregorian Calendar uses a cutover in 1582. JS Date has none.
let millisecondsPerDay = 86_400_000

func floorMod(_ n: Int, _ divisor: Int) -> Int {
    let remainder = n % divisor
    return remainder < 0 ? remainder + divisor : remainder
}

private func floorDiv(_ n: Int, _ divisor: Int) -> Int {
    (n - floorMod(n, divisor)) / divisor
}

private func padded(_ value: Int, _ width: Int) -> String {
    let digits = String(value)
    return String(repeating: "0", count: max(0, width - digits.count)) + digits
}

struct CivilDate {
    static let dayLimit = 100_000_000
    let year: Int
    let month: Int
    let date: Int

    // Split at March 1 so the leap day is the last day of a 400-year cycle.
    static func days(year: Int, month: Int, date: Int) -> Int {
        let y = month <= 2 ? year - 1 : year
        let era = floorDiv(y, 400)
        let yoe = y - era * 400
        let marchMonth = month + (month > 2 ? -3 : 9)
        let doy = (153 * marchMonth + 2) / 5 + date - 1
        return era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
    }

    init(day: Int) {
        let shifted = day + 719_468
        let era = floorDiv(shifted, 146_097)
        let doe = shifted - era * 146_097
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
        let marchMonth = (5 * doy + 2) / 153
        date = doy - (153 * marchMonth + 2) / 5 + 1
        month = marchMonth + (marchMonth < 10 ? 3 : -9)
        year = era * 400 + yoe + (month <= 2 ? 1 : 0)
    }

    /// Date.UTC treats an input year of 0 through 99 as 1900 through 1999.
    var utcYear: Int { (0...99).contains(year) ? 1900 + year : year }

    var iso: String {
        let y = (0...9999).contains(year) ? padded(year, 4) : (year < 0 ? "-" : "+") + padded(abs(year), 6)
        return y + "-" + padded(month, 2) + "-" + padded(date, 2)
    }

    static func inRange(_ day: Int) -> Bool { (-dayLimit...dayLimit).contains(day) }

    static func isoDay(_ day: Double) throws -> String {
        guard let day = Int(exactly: day), inRange(day) else { throw FunctionError("Invalid Date") }
        return String(CivilDate(day: day).iso.prefix(10))
    }

    func addingMonths(_ count: Double) throws -> String {
        // No two valid Dates differ by even seven million months. Check
        // before narrowing an arbitrarily large (but finite) double to Int.
        guard abs(count) <= 7_000_000 else { throw FunctionError("Invalid Date") }
        let target = month - 1 + Int(count)
        let y = utcYear + floorDiv(target, 12)
        let m = floorMod(target, 12) + 1
        let end = Self.days(year: m == 12 ? y + 1 : y, month: m == 12 ? 1 : m + 1, date: 0)
        guard Self.inRange(end) else { throw FunctionError("Invalid Date") }
        let last = Self(day: end).date
        return try Self.isoDay(Double(Self.days(year: y, month: m, date: min(date, last))))
    }

    func months(until other: Self) -> Int {
        let count = (other.year - year) * 12 + other.month - month
        if count > 0 && other.date < date { return count - 1 }
        if count < 0 && other.date > date { return count + 1 }
        return count
    }
}

/// Milliseconds on either the UTC or viewer's civil timeline.
struct CivilTime {
    let day: Int
    let date: CivilDate
    let hour: Int
    let minute: Int
    let second: Int
    let millisecond: Int

    init(milliseconds: Int) {
        day = floorDiv(milliseconds, millisecondsPerDay)
        date = CivilDate(day: day)
        let time = floorMod(milliseconds, millisecondsPerDay)
        hour = time / 3_600_000
        minute = time / 60_000 % 60
        second = time / 1000 % 60
        millisecond = time % 1000
    }

    var iso: String {
        date.iso + "T" + padded(hour, 2) + ":" + padded(minute, 2) + ":" + padded(second, 2)
            + "." + padded(millisecond, 3) + "Z"
    }

    private static let months = [1: "January", 2: "February", 3: "March", 4: "April", 5: "May", 6: "June",
                                 7: "July", 8: "August", 9: "September", 10: "October", 11: "November", 12: "December"]
    private static let weekdays = [0: "Sunday", 1: "Monday", 2: "Tuesday", 3: "Wednesday",
                                   4: "Thursday", 5: "Friday", 6: "Saturday"]

    func format(style: String) -> String {
        let year = date.year > 0 ? date.year : 1 - date.year
        let month = Self.months[date.month, default: ""]
        let weekday = Self.weekdays[floorMod(day + 4, 7), default: ""]
        let short = "\(month.prefix(3)) \(date.date)"
        let clock = "\(hour % 12 == 0 ? 12 : hour % 12):\(padded(minute, 2))"
        let period = hour < 12 ? " AM" : " PM"
        switch style {
        case "date": return "\(short), \(year)"
        case "short": return short
        case "long": return "\(weekday), \(month) \(date.date), \(year)"
        case "weekday": return String(weekday.prefix(3))
        case "month": return "\(month.prefix(3)) \(year)"
        case "time": return clock + period
        case "datetime": return short + " at " + clock + period
        default: return "\(date.month)/\(date.date)/\(year), \(clock):\(padded(second, 2))" + period
        }
    }
}
