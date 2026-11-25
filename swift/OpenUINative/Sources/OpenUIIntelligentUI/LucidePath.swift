// Reads only the SVG primitives and path commands present in the bundled
// Lucide artwork. Model input can select artwork but never supplies SVG.
import Foundation
import SwiftUI

struct LucideShape {
    let path: Path
    let filled: Bool

    static func read(_ markup: String) -> [Self]? {
        // At the pinned release the longest glyph is 808 bytes.
        guard markup.utf8.prefix(1_025).count <= 1_024 else { return nil }
        let reader = LucideShapeReader()
        let parser = XMLParser(data: Data("<svg>\(markup)</svg>".utf8))
        parser.delegate = reader
        guard parser.parse(), !reader.shapes.isEmpty else { return nil }
        return reader.shapes
    }
}

private final class LucideShapeReader: NSObject, XMLParserDelegate {
    var shapes: [LucideShape] = []

    func parser(_ parser: XMLParser, didStartElement tag: String, namespaceURI: String?,
                qualifiedName: String?, attributes: [String: String]) {
        guard tag != "svg" else { return }
        guard let path = Self.path(tag, attributes), !path.isEmpty else {
            parser.abortParsing()
            return
        }
        shapes.append(LucideShape(path: Path(path), filled: attributes["fill"] == "currentColor"))
    }

    private static func path(_ tag: String, _ attributes: [String: String]) -> CGPath? {
        func number(_ key: String) -> CGFloat? {
            guard let text = attributes[key], let number = Double(text), number.isFinite else { return nil }
            return CGFloat(number)
        }

        switch tag {
        case "path":
            return attributes["d"].flatMap(LucidePath.read)
        case "line":
            guard let x1 = number("x1"), let y1 = number("y1"),
                  let x2 = number("x2"), let y2 = number("y2") else { return nil }
            let path = CGMutablePath()
            path.move(to: CGPoint(x: x1, y: y1))
            path.addLine(to: CGPoint(x: x2, y: y2))
            return path
        case "circle", "ellipse":
            guard let x = number("cx"), let y = number("cy"),
                  let rx = number(tag == "circle" ? "r" : "rx"),
                  let ry = number(tag == "circle" ? "r" : "ry") else { return nil }
            return CGPath(ellipseIn: CGRect(x: x - rx, y: y - ry, width: 2 * rx, height: 2 * ry), transform: nil)
        case "rect":
            guard let x = number("x"), let y = number("y"),
                  let width = number("width"), let height = number("height") else { return nil }
            let rect = CGRect(x: x, y: y, width: width, height: height)
            let rx = number("rx"), ry = number("ry")
            let cornerX = min(rx ?? ry ?? 0, width / 2), cornerY = min(ry ?? rx ?? 0, height / 2)
            if cornerX == 0 || cornerY == 0 { return CGPath(rect: rect, transform: nil) }
            return CGPath(roundedRect: rect, cornerWidth: cornerX, cornerHeight: cornerY, transform: nil)
        case "polygon", "polyline":
            guard let points = attributes["points"] else { return nil }
            var input = LucidePathInput(points)
            guard let first = input.point() else { return nil }
            let path = CGMutablePath()
            path.move(to: first)
            while !input.atEnd {
                guard let next = input.point() else { return nil }
                path.addLine(to: next)
            }
            if tag == "polygon" { path.closeSubpath() }
            return path
        default:
            return nil
        }
    }
}

// SVG decimals can omit separators before signs and decimal points. Arc
// flags are single digits and can touch each other and the next coordinate.
private struct LucidePathInput {
    private var remaining: Substring

    init(_ source: String) { remaining = Substring(source) }

    private mutating func skipSeparators() {
        remaining = remaining.drop(while: { $0 == "," || $0.isWhitespace })
    }

    var atEnd: Bool { remaining.allSatisfy { $0 == "," || $0.isWhitespace } }

    mutating func command() -> Character? {
        skipSeparators()
        guard let first = remaining.first, first.isASCII, first.isLetter else { return nil }
        return remaining.popFirst()
    }

    mutating func number() -> CGFloat? {
        skipSeparators()
        let start = remaining
        if remaining.first == "-" || remaining.first == "+" { remaining = remaining.dropFirst() }
        remaining = remaining.drop(while: \.isNumber)
        if remaining.first == "." { remaining = remaining.dropFirst().drop(while: \.isNumber) }
        let token = start.prefix(start.count - remaining.count)
        guard let value = Double(token), value.isFinite else { return nil }
        return CGFloat(value)
    }

    mutating func point(from origin: CGPoint = .zero) -> CGPoint? {
        guard let x = number(), let y = number() else { return nil }
        return CGPoint(x: origin.x + x, y: origin.y + y)
    }

    mutating func flag() -> Bool? {
        skipSeparators()
        switch remaining.popFirst() {
        case "0": return false
        case "1": return true
        default: return nil
        }
    }
}

private enum LucidePath {
    private enum Control {
        case cubic(CGPoint)
        case quadratic(CGPoint)
    }

    static func read(_ data: String) -> CGPath? {
        var input = LucidePathInput(data)
        let path = CGMutablePath()
        var command: Character?
        var current = CGPoint.zero
        var start = CGPoint.zero
        var control: Control?
        while !input.atEnd {
            if let next = input.command() { command = next }
            guard let op = command else { return nil }
            let origin = op.isLowercase ? current : .zero
            let previous = control
            control = nil

            switch op {
            case "M", "m":
                guard let end = input.point(from: origin) else { return nil }
                path.move(to: end)
                current = end
                start = end
                command = op == "m" ? "l" : "L"
            case "L", "l":
                guard let end = input.point(from: origin) else { return nil }
                path.addLine(to: end)
                current = end
            case "H", "h":
                guard let x = input.number() else { return nil }
                current.x = origin.x + x
                path.addLine(to: current)
            case "V", "v":
                guard let y = input.number() else { return nil }
                current.y = origin.y + y
                path.addLine(to: current)
            case "C", "c", "S", "s":
                let first: CGPoint
                if op == "C" || op == "c" {
                    guard let explicit = input.point(from: origin) else { return nil }
                    first = explicit
                } else if case let .cubic(point)? = previous {
                    first = reflected(point, around: current)
                } else {
                    first = current
                }
                guard let second = input.point(from: origin), let end = input.point(from: origin) else { return nil }
                path.addCurve(to: end, control1: first, control2: second)
                current = end
                control = .cubic(second)
            case "Q", "q", "T", "t":
                let first: CGPoint
                if op == "Q" || op == "q" {
                    guard let explicit = input.point(from: origin) else { return nil }
                    first = explicit
                } else if case let .quadratic(point)? = previous {
                    first = reflected(point, around: current)
                } else {
                    first = current
                }
                guard let end = input.point(from: origin) else { return nil }
                path.addQuadCurve(to: end, control: first)
                current = end
                control = .quadratic(first)
            case "A", "a":
                guard let rx = input.number(), let ry = input.number(), let rotation = input.number(),
                      let large = input.flag(), let sweep = input.flag(), let end = input.point(from: origin)
                else { return nil }
                addArc(to: path, from: current, to: end, radii: CGSize(width: abs(rx), height: abs(ry)),
                       rotation: rotation, large: large, sweep: sweep)
                current = end
            case "Z", "z":
                path.closeSubpath()
                current = start
                command = nil
            default:
                return nil
            }
        }
        return path
    }

    private static func reflected(_ point: CGPoint, around center: CGPoint) -> CGPoint {
        CGPoint(x: 2 * center.x - point.x, y: 2 * center.y - point.y)
    }

    // Convert SVG's endpoint form to a unit arc transformed onto its ellipse.
    // Enlarge impossible radii as SVG does before choosing the centre.
    private static func addArc(to path: CGMutablePath, from start: CGPoint, to end: CGPoint, radii: CGSize,
                               rotation: CGFloat, large: Bool, sweep: Bool) {
        guard start != end else { return }
        guard radii.width > 0, radii.height > 0 else {
            path.addLine(to: end)
            return
        }
        let phi = rotation * .pi / 180
        let c = cos(phi), s = sin(phi)
        let dx = (start.x - end.x) / 2, dy = (start.y - end.y) / 2
        let x = c * dx + s * dy, y = -s * dx + c * dy
        let scale = max(1, hypot(x / radii.width, y / radii.height))
        let rx = radii.width * scale, ry = radii.height * scale
        let u = x / rx, v = y / ry
        let distance = u * u + v * v
        let sign: CGFloat = large == sweep ? -1 : 1
        let factor = sign * sqrt(max(0, 1 / distance - 1))
        let centerX = factor * v, centerY = -factor * u

        let startAngle = atan2(v - centerY, u - centerX)
        let endAngle = atan2(-v - centerY, -u - centerX)
        var delta = endAngle - startAngle
        if sweep && delta < 0 { delta += 2 * .pi }
        if !sweep && delta > 0 { delta -= 2 * .pi }
        let transform = CGAffineTransform(
            a: rx * c, b: rx * s, c: -ry * s, d: ry * c,
            tx: (start.x + end.x) / 2 + rx * c * centerX - ry * s * centerY,
            ty: (start.y + end.y) / 2 + rx * s * centerX + ry * c * centerY)
        path.addRelativeArc(center: .zero, radius: 1, startAngle: startAngle, delta: delta, transform: transform)
    }
}
