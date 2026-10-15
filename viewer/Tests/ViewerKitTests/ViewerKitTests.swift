import SwiftUI
import Testing
@testable import ViewerKit

@Suite struct JSONReading {
    @Test func objectKeysKeepTheirOrder() throws {
        let value = try JSONValue.parse(#"{"b": 1, "a": [true, null], "c": {"z": "x", "y": -2.5e1}}"#)
        guard case let .object(entries) = value else { Issue.record("not an object"); return }
        #expect(entries.map(\.key) == ["b", "a", "c"])
        #expect(value["c"]?.compact == #"{"z":"x","y":-25}"#)
    }

    @Test func escapesAndSurrogatePairsDecode() throws {
        let value = try JSONValue.parse(#""a\"b\\c\né😀""#)
        #expect(value.string == "a\"b\\c\né😀")
    }

    @Test func aLoneSurrogateBecomesTheReplacementCharacter() throws {
        #expect(try JSONValue.parse(#""x\ud800y""#).string == "x\u{FFFD}y")
    }

    @Test func malformedTextIsRejected() {
        #expect(throws: JSONError.self) { try JSONValue.parse("[1, 2") }
        #expect(throws: JSONError.self) { try JSONValue.parse("{} x") }
    }
}

@Suite struct TheBinding {
    @Test func aSchemaThatIsNotJSONThrows() {
        #expect(throws: (any Error).self) { try Library(schema: #"{"$defs":"#) }
    }

    @Test func elementsPropsAndExpressionsAreSortedApart() throws {
        let schema = #"{"$defs": {"Stack": {"properties": {"children": {"type": "array"}}}, "Text": {"properties": {"value": {"type": "string"}}}}}"#
        let result = try parseOneShot("root = Stack([a, b])\na = Text(\"hi\")\nb = Text($x)", library: Library(schema: schema))
        let root = try #require(result.root)
        #expect(root.typeName == "Stack")
        guard case let .list(children) = root.props.first?.value else { Issue.record("no children"); return }
        guard case let .element(a) = children[0], case let .element(b) = children[1] else {
            Issue.record("children aren't elements"); return
        }
        #expect(a.props.first?.value.summary == #""hi""#)
        guard case .expression = b.props.first?.value else { Issue.record("$x isn't an expression"); return }
    }
}

@MainActor @Test func aCorpusCaseDecodesAndDrawsOffscreen() throws {
    let corpus = try Corpus.load()
    let c = try #require(corpus.cases.first { $0.name == "bench/dashboard" })
    let result = try parseOneShot(corpus.program(of: c), library: Library(schema: corpus.schema(of: c)), root: c.root)
    let root = try #require(result.root)
    let image = try #require(ImageRenderer(content: ElementBox(element: root)).nsImage)
    #expect(image.size.width > 100 && image.size.height > 100)
}
