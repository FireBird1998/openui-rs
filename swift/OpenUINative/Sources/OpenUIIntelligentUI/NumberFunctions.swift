// Numbers: maths, money and units. Upstream's lib/functions/numbers.ts, declared
// in its order. Each returns null until its ticket gives it an implementation.
import OpenUINative

extension IntelligentUI {
    static let numberFunctions: [DefinedFunction] = [
        function("Pow", "base raised to exponent", [
            .init("base", .number),
            .init("exponent", .number),
        ], returns: .number),
        function("Sqrt", "Square root; null for a negative number", [
            .init("number", .number),
        ], returns: .nullable(.number)),
        function("Random", "Random number from min to max (integer true: a whole number, max included); draws anew each time, so call it inside @Set", [
            .init("min", .number),
            .init("max", .number),
            .init("integer", .boolean, optional: true),
        ], returns: .number),
        function("Clamp", "number kept between min and max", [
            .init("number", .number),
            .init("min", .number),
            .init("max", .number),
        ], returns: .number),
        function("Percent", "part / total * 100, e.g. @Percent(3, 4) is 75; 0 when total is 0", [
            .init("part", .number),
            .init("total", .number),
        ], returns: .number),
        function("Lerp", "The value a fraction t of the way from a to b", [
            .init("a", .number),
            .init("b", .number),
            .init("t", .number),
        ], returns: .number),
        function("Allocate", "Splits total in proportion to weights, rounded to decimals (default 2), so the parts add up to exactly total, e.g. @Allocate(100, [1, 1, 1]) is [33.34, 33.33, 33.33]", [
            .init("total", .number),
            .init("weights", .numbers),
            .init("decimals", .number, optional: true),
        ], returns: .numbers),
        function("Payment", "Payment per period that repays principal over periods at rate percent per period, e.g. a 30-year loan at 6% a year: @Payment(300000, 6 / 12, 360)", [
            .init("principal", .number),
            .init("rate", .number),
            .init("periods", .number),
        ], returns: .number),
        function("Convert", "value from one unit to another: mm cm m km in ft yd mi; mg g kg oz lb st; ml l tsp tbsp floz cup pt qt gal (US); m/s km/h mph kn; m2 km2 ft2 ha acre; c f k", [
            .init("value", .number),
            .init("from", .string),
            .init("to", .string),
        ], returns: .number),
    ]
}
