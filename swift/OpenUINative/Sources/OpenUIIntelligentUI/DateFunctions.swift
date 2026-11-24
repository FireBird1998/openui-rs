// Dates: the clock, formatting and calendar arithmetic. Upstream's lib/functions/dates.ts, declared
// in its order.
import Foundation
import OpenUINative

extension IntelligentUI {
    static func dateFunctions(clock: @escaping @Sendable () -> Date, zone: TimeZone) -> [DefinedFunction] {
        let dates = DateFunctions(zone: zone)
        return [
            function("Now", "The current date and time as an ISO string", [
            ], returns: .string) { _ in
                try .string(DateFunctions.isoInstant(clock()))
            },
            function("Today", #"Today's date in the viewer's time zone, as "YYYY-MM-DD""#, [
            ], returns: .string) { _ in
                try .string(dates.isoToday(clock()))
            },
            function("FormatDate", #"A date as text: "date" Oct 9, 2026 (default), "short" Oct 9, "long" Friday, October 9, 2026, "weekday" Fri, "month" Oct 2026, "time" 9:30 PM, "datetime", "iso" 2026-10-09. Dates are "YYYY-MM-DD", ISO or a timestamp; "time" and "datetime" need a timestamp or an ISO date-time, as a plain date has no time"#, [
                .init("date", .date),
                .init("style", .words(["date", "short", "long", "weekday", "month", "time", "datetime", "iso"]), optional: true),
            ], returns: .string) { args in
                let parsed = try dates.parse(args.itemArgument("date"))
                let style = args["style"] ?? .string("date")
                if style.string == "iso" { return try .string(parsed.isoDay()) }
                return try .string(parsed.local.format(style: ListFunctions.text(style)))
            },
            function("AddDays", #"The date days later (negative: earlier), as "YYYY-MM-DD""#, [
                .init("date", .date),
                .init("days", .number),
            ], returns: .string) { args in
                let parsed = try dates.parse(args.itemArgument("date"))
                let offset = try args.numberArgument("days").rounded(.towardZero)
                return try .string(CivilDate.isoDay(Double(parsed.validDay()) + offset))
            },
            function("AddMonths", #"The date months later (negative: earlier), kept within the month (Jan 31 + 1 is Feb 28), as "YYYY-MM-DD"; 12 months is a year"#, [
                .init("date", .date),
                .init("months", .number),
            ], returns: .string) { args in
                let parsed = try dates.parse(args.itemArgument("date"))
                return try .string(CivilDate(day: parsed.validDay()).addingMonths(args.numberArgument("months")))
            },
            function("DaysBetween", "Calendar days from one date to another (negative if to is earlier)", [
                .init("from", .date),
                .init("to", .date),
            ], returns: .number) { args in
                let to = try dates.parse(args.itemArgument("to"))
                let from = try dates.parse(args.itemArgument("from"))
                guard let end = to.day, let start = from.day else { return .null }
                return .number(Double(end - start))
            },
            function("MonthsBetween", "Whole months from one date to another (negative if to is earlier); @Floor(@MonthsBetween(birthday, @Today()) / 12) is an age", [
                .init("from", .date),
                .init("to", .date),
            ], returns: .number) { args in
                let from = try dates.parse(args.itemArgument("from"))
                let to = try dates.parse(args.itemArgument("to"))
                guard let start = from.day, let end = to.day else { return .null }
                return .number(Double(CivilDate(day: start).months(until: CivilDate(day: end))))
            },
            function("StartOf", #"First day of the date's "week" (Monday), "month" or "year", as "YYYY-MM-DD""#, [
                .init("date", .date),
                .init("unit", .words(["week", "month", "year"])),
            ], returns: .string) { args in
                let day = try dates.parse(args.itemArgument("date")).validDay()
                let unit = args["unit"]?.string
                if unit == "week" { return try .string(CivilDate.isoDay(Double(day - floorMod(day + 3, 7)))) }
                let date = CivilDate(day: day)
                return try .string(CivilDate.isoDay(Double(CivilDate.days(year: date.utcYear,
                                                           month: unit == "month" ? date.month : 1, date: 1))))
            },
        ]
    }
}

struct ParsedDate {
    let day: Int?
    let local: CivilTime

    func validDay() throws -> Int {
        guard let day else { throw FunctionError("Invalid Date") }
        return day
    }

    func isoDay() throws -> String { try CivilDate.isoDay(Double(validDay())) }
}

struct DateFunctions {
    let zone: TimeZone

    static func isoInstant(_ date: Date) throws -> String {
        guard let ms = DateParser.clip(date.timeIntervalSince1970 * 1000) else { throw FunctionError("Invalid Date") }
        return CivilTime(milliseconds: ms).iso
    }

    func isoToday(_ date: Date) throws -> String { try parse(.number(date.timeIntervalSince1970 * 1000)).isoDay() }

    func parse(_ value: JSONValue) throws -> ParsedDate {
        let stamp: Int?
        switch value {
        case let .string(text):
            if let (y, m, d) = DateParser.plainDate(text) {
                let day = CivilDate.days(year: y < 100 ? 1900 + y : y, month: m, date: d)
                let local = CivilTime(milliseconds: day * millisecondsPerDay)
                guard local.date.month == m, local.date.date == d else { throw FunctionError("invalid date \"\(text)\"") }
                return ParsedDate(day: day, local: local)
            }
            stamp = DateParser.milliseconds(text, in: zone)
        case let .number(number): stamp = DateParser.clip(number)
        case let .bool(flag): stamp = flag ? 1 : 0
        case .null: stamp = 0
        default: stamp = try DateParser.milliseconds(ListFunctions.text(value), in: zone)
        }
        guard let stamp else { throw FunctionError("invalid date \"\(try ListFunctions.text(value))\"") }
        let local = CivilTime(milliseconds: stamp + zone.offsetMilliseconds(at: stamp))
        let day = CivilDate.days(year: local.date.utcYear, month: local.date.month, date: local.date.date)
        return ParsedDate(day: CivilDate.inRange(day) ? day : nil, local: local)
    }
}
