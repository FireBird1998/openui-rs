// Data: shaping Query rows for tables and charts. Upstream's lib/functions/data.ts, declared
// in its order. Each returns null until its ticket gives it an implementation.
import OpenUINative

extension IntelligentUI {
    static let dataFunctions: [DefinedFunction] = [
        function("GroupBy", #"One row per distinct key, in first-seen order; value is agg of field (default "sum"), or the row count without a field"#, [
            .init("rows", .list),
            .init("key", .string),
            .init("field", .string, optional: true),
            .init("agg", .agg, optional: true),
        ], returns: .array(.object([.init("key", .date), .init("value", .number)]))),
        function("Pivot", "One row per rowKey value, under key, with a column per colKey value holding agg of field, for stacked or grouped charts", [
            .init("rows", .list),
            .init("rowKey", .string),
            .init("colKey", .string),
            .init("field", .string, optional: true),
            .init("agg", .agg, optional: true),
        ], returns: .array(.record(.date))),
        function("Top", #"The n rows with the largest field ("asc": the smallest)"#, [
            .init("rows", .list),
            .init("field", .string),
            .init("n", .number),
            .init("direction", .words(["desc", "asc"]), optional: true),
        ], returns: .list),
        function("Between", #"Rows whose field is from min to max, both included; compares numbers, dates by day ("YYYY-MM-DD", ISO or a timestamp), or text. Leave min or max null for an open end"#, [
            .init("rows", .list),
            .init("field", .string),
            .init("min", .any, optional: true),
            .init("max", .any, optional: true),
        ], returns: .list),
        function("Bucket", #"Rows per day, week (from Monday) or month of dateField, in date order, empty periods 0 for sum and count; key is "YYYY-MM-DD", or "YYYY-MM" for months"#, [
            .init("rows", .list),
            .init("dateField", .string),
            .init("unit", .words(["day", "week", "month"])),
            .init("field", .string, optional: true),
            .init("agg", .agg, optional: true),
        ], returns: .array(.object([.init("key", .date), .init("value", .number)]))),
        function("Cumulative", #"Copy of the rows with field replaced by its running total (field "" for a list of numbers)"#, [
            .init("rows", .list),
            .init("field", .string),
        ], returns: .list),
        function("MovingAvg", #"Copy of the rows with field replaced by the average of its last window values (field "" for a list of numbers)"#, [
            .init("rows", .list),
            .init("field", .string),
            .init("window", .number),
        ], returns: .list),
        function("Change", "Percent change from previous to current, e.g. @Change(120, 100) is 20; null when previous is 0", [
            .init("current", .number),
            .init("previous", .number),
        ], returns: .nullable(.number)),
        function("Histogram", "Counts numbers into equal-width bins as {key, value} rows: key is the bin's start, value the count; min and max fix the range, default the data's", [
            .init("values", .numbers),
            .init("bins", .number),
            .init("min", .number, optional: true),
            .init("max", .number, optional: true),
        ], returns: .array(.object([.init("key", .string), .init("value", .number)]))),
        function("Stats", "Summary of numbers, all zeros for an empty list", [
            .init("values", .numbers),
        ], returns: .object([.init("count", .number), .init("min", .number), .init("max", .number), .init("mean", .number), .init("median", .number), .init("stdev", .number)])),
    ]
}
