// Numbers: maths, money and units. Upstream's lib/functions/numbers.ts, declared
// in its order.
import Darwin
import OpenUINative

extension IntelligentUI {
    static func numberFunctions(random: @escaping @Sendable () -> Double) -> [DefinedFunction] {
        [
            function("Pow", "base raised to exponent", [
                .init("base", .number),
                .init("exponent", .number),
            ], returns: .number) { args in
                try NumberFunctions.finite(NumberFunctions.power(args.numberArgument("base"), args.numberArgument("exponent")))
            },
            function("Sqrt", "Square root; null for a negative number", [
                .init("number", .number),
            ], returns: .nullable(.number)) { args in
                let n = try args.numberArgument("number")
                return n < 0 ? .null : try NumberFunctions.finite(n.squareRoot())
            },
            function("Random", "Random number from min to max (integer true: a whole number, max included); draws anew each time, so call it inside @Set", [
                .init("min", .number),
                .init("max", .number),
                .init("integer", .boolean, optional: true),
            ], returns: .number) { args in
                guard let a = args["min"]?.number, let b = args["max"]?.number, a.isFinite, b.isFinite else {
                    throw FunctionError("min and max must be numbers")
                }
                let lo = min(a, b), hi = max(a, b)
                if args["integer"]?.bool == true {
                    let first = lo.rounded(.up), last = hi.rounded(.down)
                    guard first <= last else { throw FunctionError("no whole number between min and max") }
                    let draw = try NumberFunctions.draw(random)
                    return NumberFunctions.jsonNumber(first + (draw * (last - first + 1)).rounded(.down))
                }
                return try NumberFunctions.jsonNumber(lo + NumberFunctions.draw(random) * (hi - lo))
            },
            function("Clamp", "number kept between min and max", [
                .init("number", .number),
                .init("min", .number),
                .init("max", .number),
            ], returns: .number) { args in
                let n = try args.numberArgument("number")
                let a = try args.numberArgument("min")
                let b = try args.numberArgument("max")
                return .number(min(max(n, min(a, b)), max(a, b)))
            },
            function("Percent", "part / total * 100, e.g. @Percent(3, 4) is 75; 0 when total is 0", [
                .init("part", .number),
                .init("total", .number),
            ], returns: .number) { args in
                let part = try args.numberArgument("part")
                let total = try args.numberArgument("total")
                return total == 0 ? .number(0) : try NumberFunctions.finite((part / total) * 100)
            },
            function("Lerp", "The value a fraction t of the way from a to b", [
                .init("a", .number),
                .init("b", .number),
                .init("t", .number),
            ], returns: .number) { args in
                let a = try args.numberArgument("a")
                let b = try args.numberArgument("b")
                let t = try args.numberArgument("t")
                return try NumberFunctions.finite(a + (b - a) * t)
            },
            function("Allocate", "Splits total in proportion to weights, rounded to decimals (default 2), so the parts add up to exactly total, e.g. @Allocate(100, [1, 1, 1]) is [33.34, 33.33, 33.33]", [
                .init("total", .number),
                .init("weights", .numbers),
                .init("decimals", .number, optional: true),
            ], returns: .numbers, implementation: NumberFunctions.allocate),
            function("Payment", "Payment per period that repays principal over periods at rate percent per period, e.g. a 30-year loan at 6% a year: @Payment(300000, 6 / 12, 360)", [
                .init("principal", .number),
                .init("rate", .number),
                .init("periods", .number),
            ], returns: .number) { args in
                let principal = try args.numberArgument("principal")
                let rate = try args.numberArgument("rate") / 100
                let periods = try args.numberArgument("periods")
                guard periods > 0 else { throw FunctionError("periods must be positive") }
                return try NumberFunctions.finite(rate == 0 ? principal / periods
                    : (principal * rate) / (1 - NumberFunctions.power(1 + rate, -periods)))
            },
            function("Convert", "value from one unit to another: mm cm m km in ft yd mi; mg g kg oz lb st; ml l tsp tbsp floz cup pt qt gal (US); m/s km/h mph kn; m2 km2 ft2 ha acre; c f k", [
                .init("value", .number),
                .init("from", .string),
                .init("to", .string),
            ], returns: .number, implementation: NumberFunctions.convert),
        ]
    }
}

enum NumberFunctions {
    private enum Kind { case length, mass, volume, speed, area }
    private static let units: [String: (kind: Kind, factor: Double)] = [
        "mm": (.length, 0.001), "cm": (.length, 0.01), "m": (.length, 1), "km": (.length, 1000),
        "in": (.length, 0.0254), "ft": (.length, 0.3048), "yd": (.length, 0.9144), "mi": (.length, 1609.344),
        "mg": (.mass, 1e-6), "g": (.mass, 0.001), "kg": (.mass, 1), "oz": (.mass, 0.028349523125),
        "lb": (.mass, 0.45359237), "st": (.mass, 6.35029318),
        "ml": (.volume, 0.001), "l": (.volume, 1), "tsp": (.volume, 0.00492892159375),
        "tbsp": (.volume, 0.0147867647812), "floz": (.volume, 0.0295735295625), "cup": (.volume, 0.2365882365),
        "pt": (.volume, 0.473176473), "qt": (.volume, 0.946352946), "gal": (.volume, 3.785411784),
        "m/s": (.speed, 1), "km/h": (.speed, 1 / 3.6), "mph": (.speed, 0.44704), "kn": (.speed, 0.514444),
        "m2": (.area, 1), "km2": (.area, 1e6), "ft2": (.area, 0.09290304),
        "ha": (.area, 10000), "acre": (.area, 4046.8564224),
    ]

    static func convert(_ args: JSONValue) throws -> JSONValue {
        let value = try args.numberArgument("value")
        let from = try args.stringArgument("from"), to = try args.stringArgument("to")
        let a = String(decoding: JavaScript.trim(from), as: UTF16.self).lowercased()
        let b = String(decoding: JavaScript.trim(to), as: UTF16.self).lowercased()
        let temperatures = ["c", "f", "k"]
        if temperatures.contains(a), temperatures.contains(b) {
            let celsius = switch a {
            case "f": (value - 32) * 5 / 9
            case "k": value - 273.15
            default: value
            }
            switch b {
            case "f": return try finite(celsius * 9 / 5 + 32)
            case "k": return try finite(celsius + 273.15)
            default: return try finite(celsius)
            }
        }
        // These two lowercase keys resolve on Object.prototype. Both have
        // undefined kind and factor, so converting between them reaches finite(NaN).
        let inherited = ["constructor", "__proto__"]
        if inherited.contains(a), inherited.contains(b) { return try finite(.nan) }
        guard let source = units[a], let target = units[b], source.kind == target.kind else {
            throw FunctionError("cannot convert \(from) to \(to)")
        }
        return try finite((value * source.factor) / target.factor)
    }

    static func allocate(_ args: JSONValue) throws -> JSONValue {
        let total = try args.numberArgument("total")
        let decimals = args["decimals"] == nil ? 2 : try args.numberArgument("decimals")
        guard let items = args["weights"]?.array, !items.isEmpty else {
            throw FunctionError("weights must be zero or more")
        }
        let weights = try items.map { item in
            guard let weight = item.number, weight.isFinite, weight >= 0 else {
                throw FunctionError("weights must be zero or more")
            }
            return weight
        }
        let sum = weights.reduce(0, +)
        guard sum != 0 else { return .array(Array(repeating: .number(0), count: weights.count)) }
        let scale = power(10, min(6, max(0, JavaScript.round(decimals))))
        let units = JavaScript.round(abs(total) * scale)
        let shares = weights.map { units * $0 / sum }
        var parts = shares.map { $0.rounded(.down) }
        let left = units - parts.reduce(0, +)

        // Upstream awards at most one unit per weight. If left is NaN,
        // its stopping condition (left <= 0) never holds and every part gains one.
        if left.isNaN || left >= Double(parts.count) {
            for index in parts.indices { parts[index] += 1 }
        } else if left > 0, let extra = Int(exactly: left) {
            // A finite leftover implies all shares are finite.
            let remainders = zip(shares, parts).map { $0 - $1 }
            awardAllocationBonuses(extra, remainders: remainders, parts: &parts)
        }
        return .array(parts.map { part in
            jsonNumber((total < 0 ? -part : part) / scale)
        })
    }

    static func draw(_ random: @Sendable () -> Double) throws -> Double {
        let value = random()
        guard (0..<1).contains(value) else { throw FunctionError("random draw must be in [0, 1)") }
        return value
    }

    /// A returned number crosses the binding as JSON.stringify would write it.
    static func jsonNumber(_ value: Double) -> JSONValue {
        value.isFinite ? .number(value) : .null
    }

    /// Upstream explicitly throws inside finite() before it returns a value.
    static func finite(_ value: Double) throws -> JSONValue {
        guard value.isFinite else { throw FunctionError("result is not a finite number") }
        return .number(value)
    }

    /// Bun's Math.pow uses square roots for the half powers and repeated
    /// squaring for integral exponents from 0 through 1000. Darwin's pow
    /// can round differently on those paths.
    static func power(_ base: Double, _ exponent: Double) -> Double {
        if exponent == 0.5 { return base == 0 ? 0 : base.squareRoot() }
        if exponent == -0.5 { return base == 0 ? .infinity : 1 / base.squareRoot() }
        guard (0...1000).contains(exponent), var n = Int(exactly: exponent) else {
            return Darwin.pow(base, exponent)
        }
        var factor = base
        var product = 1.0
        while n > 0 {
            if n & 1 != 0 { product *= factor }
            n >>= 1
            factor *= factor
        }
        return product
    }
}

/// Give one unit to each of the largest remainders, breaking ties in input order.
/// Subtracting the floor gives finite remainders with a clear sign bit (including
/// zero), so their unsigned binary64 bit patterns have the same order as the values.
func awardAllocationBonuses(_ count: Int, remainders: some Collection<Double>, parts: inout [Double]) {
    var prefix: UInt64 = 0
    var mask: UInt64 = 0
    var needed = count
    for shift in stride(from: 56, through: 0, by: -8) {
        var buckets = Array(repeating: 0, count: 256)
        for remainder in remainders {
            let bits = remainder.bitPattern
            if bits & mask == prefix {
                buckets[Int(UInt8(truncatingIfNeeded: bits >> shift))] += 1
            }
        }
        // Discard larger buckets until the needed rank lies in this bucket.
        for byte in buckets.indices.reversed() {
            if needed <= buckets[byte] {
                prefix |= UInt64(byte) << shift
                break
            }
            needed -= buckets[byte]
        }
        mask |= UInt64(0xff) << shift
    }
    // All larger remainders win. The residual rank is the number of ties to take.
    let cutoff = Double(bitPattern: prefix)
    for (index, remainder) in zip(parts.indices, remainders) {
        if remainder > cutoff {
            parts[index] += 1
        } else if remainder == cutoff && needed > 0 {
            parts[index] += 1
            needed -= 1
        }
    }
}
