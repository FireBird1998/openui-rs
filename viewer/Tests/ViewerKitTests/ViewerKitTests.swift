import Foundation
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

@Suite struct TheCore {
    static let schema = #"{"$defs": {"Stack": {"properties": {"children": {"type": "array"}}}, "Text": {"properties": {"value": {"type": "string"}}}}}"#

    @Test func itRunsOnItsOwnThreadWithA1MBStack() async throws {
        let core = Core()
        #expect(!Core.isCurrent)
        let (onCore, stack) = try await core.run { (Core.isCurrent, Thread.current.stackSize) }
        #expect(onCore)
        #expect(stack == 1 << 20)
    }

    @Test func aSchemaThatIsNotJSONThrows() async {
        await #expect(throws: (any Error).self) { try await Core().library(schema: #"{"$defs":"#) }
    }

    @Test func elementsPropsAndExpressionsAreSortedApart() async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let step = try await core.parseOneShot("root = Stack([a, b])\na = Text(\"hi\")\nb = Text($x)", library: library)
        let root = try #require(step.result.root)
        #expect(root.typeName == "Stack")
        guard case let .list(children) = root.props.first?.value else { Issue.record("no children"); return }
        guard case let .element(a) = children[0], case let .element(b) = children[1] else {
            Issue.record("children aren't elements"); return
        }
        #expect(a.props.first?.value.summary == #""hi""#)
        guard case .expression = b.props.first?.value else { Issue.record("$x isn't an expression"); return }
    }

    @Test func aStreamEndsWhereOneShotParsingDoesOnASimpleProgram() async throws {
        let core = Core()
        let library = try await core.library(schema: Self.schema)
        let program = "root = Stack([a])\na = Text(\"hello\")"
        let session = try await core.session(library: library)
        var last: Step?
        for chunk in ReplaySource.split(program, size: 3) {
            last = try await session.push([chunk])
        }
        #expect(try await last?.json == core.parseOneShot(program, library: library).json)
    }
}

@Suite struct ReplayingAProgram {
    static let program = "a😀é\nb = X(\"ü\")"

    @Test(arguments: [1, 2, 3, 5, 100])
    func chunksComeInOrderWithTheirBoundaries(size: Int) async throws {
        var batches: [[String]] = []
        for try await batch in ReplaySource(program: Self.program, chunkSize: size).chunks() {
            batches.append(batch)
        }
        #expect(batches.count == 1)
        let chunks = batches.flatMap { $0 }
        #expect(chunks.joined() == Self.program)
        let sizes = chunks.map(\.unicodeScalars.count)
        let total = Self.program.unicodeScalars.count
        #expect(sizes.dropLast().allSatisfy { $0 == size })
        #expect(sizes.last == total - size * (chunks.count - 1))
        #expect(chunks.count == (total + size - 1) / size)
    }

    @Test func anEmptyProgramHasNoChunks() async throws {
        var count = 0
        for try await _ in ReplaySource(program: "", chunkSize: 4).chunks() { count += 1 }
        #expect(count == 0)
    }
}

@MainActor @Suite struct PlayingBack {
    func playback(cacheBudget: Int) async throws -> Playback {
        let corpus = try Corpus.load()
        let c = try #require(corpus.cases.first { $0.name == "hand/04-nested-card-mixed-args" })
        let core = Core()
        let library = try await core.library(schema: corpus.schema(of: c))
        let playback = Playback(core: core, library: library, root: c.root, cacheBudget: cacheBudget)
        await playback.load(ReplaySource(program: try corpus.program(of: c), chunkSize: 1))
        return playback
    }

    @Test func loadingShowsTheLastStep() async throws {
        let playback = try await playback(cacheBudget: 32 << 20)
        #expect(playback.position == playback.chunks.count)
        #expect(playback.step != nil)
    }

    /// Going back shows exactly the earlier step, whether it was kept (a big
    /// budget) or has to be rebuilt (a budget that keeps almost nothing).
    @Test(arguments: [32 << 20, 1])
    func steppingBackShowsExactlyTheEarlierResult(cacheBudget: Int) async throws {
        let playback = try await playback(cacheBudget: cacheBudget)
        var seen: [Int: String] = [:]
        await playback.move(to: 0)
        for k in 0...playback.chunks.count {
            await playback.move(to: k)
            seen[k] = try #require(playback.step).json
        }
        for k in [playback.chunks.count - 1, 7, 0, 30, 29, 3] where k <= playback.chunks.count {
            await playback.move(to: k)
            #expect(playback.position == k)
            #expect(playback.step?.json == seen[k], "step \(k)")
        }
    }

    @Test func changingTheChunkSizeResumesNearTheSamePlace() async throws {
        let playback = try await playback(cacheBudget: 32 << 20)
        await playback.move(to: 10)
        let program = playback.chunks.joined()
        await playback.load(ReplaySource(program: program, chunkSize: 4), resumeAt: playback.received)
        #expect(playback.position == 3) // 12 code points cover the first 10
    }
}

@MainActor @Test func aCorpusCaseDecodesAndDrawsOffscreen() async throws {
    let corpus = try Corpus.load()
    let c = try #require(corpus.cases.first { $0.name == "bench/dashboard" })
    let core = Core()
    let library = try await core.library(schema: corpus.schema(of: c))
    let step = try await core.parseOneShot(corpus.program(of: c), library: library, root: c.root)
    let root = try #require(step.result.root)
    let image = try #require(ImageRenderer(content: ElementBox(element: root)).nsImage)
    #expect(image.size.width > 100 && image.size.height > 100)
}
