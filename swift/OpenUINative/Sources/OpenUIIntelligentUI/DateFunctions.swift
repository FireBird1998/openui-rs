// Dates: the clock, formatting and calendar arithmetic. Upstream's lib/functions/dates.ts, declared
// in its order. Each returns null until its ticket gives it an implementation.
import OpenUINative

extension IntelligentUI {
    static let dateFunctions: [DefinedFunction] = [
        function("Now", "The current date and time as an ISO string", [
        ], returns: .string),
        function("Today", #"Today's date in the viewer's time zone, as "YYYY-MM-DD""#, [
        ], returns: .string),
        function("FormatDate", #"A date as text: "date" Oct 9, 2026 (default), "short" Oct 9, "long" Friday, October 9, 2026, "weekday" Fri, "month" Oct 2026, "time" 9:30 PM, "datetime", "iso" 2026-10-09. Dates are "YYYY-MM-DD", ISO or a timestamp; "time" and "datetime" need a timestamp or an ISO date-time, as a plain date has no time"#, [
            .init("date", .date),
            .init("style", .words(["date", "short", "long", "weekday", "month", "time", "datetime", "iso"]), optional: true),
        ], returns: .string),
        function("AddDays", #"The date days later (negative: earlier), as "YYYY-MM-DD""#, [
            .init("date", .date),
            .init("days", .number),
        ], returns: .string),
        function("AddMonths", #"The date months later (negative: earlier), kept within the month (Jan 31 + 1 is Feb 28), as "YYYY-MM-DD"; 12 months is a year"#, [
            .init("date", .date),
            .init("months", .number),
        ], returns: .string),
        function("DaysBetween", "Calendar days from one date to another (negative if to is earlier)", [
            .init("from", .date),
            .init("to", .date),
        ], returns: .number),
        function("MonthsBetween", "Whole months from one date to another (negative if to is earlier); @Floor(@MonthsBetween(birthday, @Today()) / 12) is an age", [
            .init("from", .date),
            .init("to", .date),
        ], returns: .number),
        function("StartOf", #"First day of the date's "week" (Monday), "month" or "year", as "YYYY-MM-DD""#, [
            .init("date", .date),
            .init("unit", .words(["week", "month", "year"])),
        ], returns: .string),
    ]
}
