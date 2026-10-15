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

    /// Two loads started back to back run one after the other; the last one wins,
    /// and the position never points past the chunks.
    @Test func overlappingLoadsEndInTheLastOnesState() async throws {
        let playback = try await playback(cacheBudget: 32 << 20)
        let program = playback.chunks.joined()
        let first = Task { await playback.load(ReplaySource(program: program, chunkSize: 2)) }
        let second = Task { await playback.load(ReplaySource(program: program, chunkSize: 5)) }
        #expect(playback.position <= playback.chunks.count)
        await first.value
        #expect(playback.position <= playback.chunks.count)
        await second.value
        #expect(playback.chunks == ReplaySource.split(program, size: 5))
        #expect(playback.position == playback.chunks.count)
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

@Suite struct TheCorpus {
    /// The reference's own one-shot result for every case decodes into the
    /// viewer's model.
    @Test func everyGoldenOneShotResultDecodes() throws {
        let corpus = try Corpus.load()
        var failures: [String] = []
        for c in corpus.cases {
            let url = corpus.directory.appendingPathComponent("golden/\(c.name).oneshot.json")
            do {
                _ = try ParseResult(json: String(contentsOf: url, encoding: .utf8), depthLimited: false, sizeLimited: false)
            } catch {
                failures.append("\(c.name): \(error)")
            }
        }
        #expect(corpus.cases.count >= 300)
        #expect(failures.isEmpty, "\(failures.joined(separator: "\n"))")
    }

    @Test func casesGroupBySourceAndFilterByName() throws {
        let corpus = try Corpus.load()
        let all = corpus.sections(matching: "")
        #expect(all.first?.title == "Hand-written")
        #expect(all.map(\.cases.count).reduce(0, +) == corpus.cases.count)
        #expect(Set(all.map(\.source)).count == all.count)
        let found = corpus.sections(matching: "DASHBOARD-X")
        #expect(found.map(\.title) == ["Synthetic"])
        #expect(found.first?.cases.count == 3)
    }
}

@Suite struct Goldens {
    @Test func fingerprintsAreTheGoldensFNV1a() {
        #expect(fingerprint("") == "811c9dc5")
        #expect(fingerprint("a") == "e40c292c")
    }

    @Test func aWrongFingerprintIsReportedAsDiffering() {
        let json = #"{"root":null}"#
        let golden = GoldenSteps(steps: ["4": [fingerprint(json), "00000000"]])
        #expect(golden.verdict(step: 1, json: json, chunking: .fixed(4)) == .matches)
        #expect(golden.verdict(step: 2, json: json, chunking: .fixed(4))
            == .differs(reference: "00000000", viewer: fingerprint(json)))
        #expect(golden.verdict(step: 0, json: json, chunking: .fixed(4)) == .nothingPushed)
        #expect(golden.verdict(step: 3, json: json, chunking: .fixed(4)) == .noGolden)
        #expect(golden.verdict(step: 1, json: json, chunking: .fixed(64)) == .noGolden)
    }

    @Test func recordedChunkingsReplayTheirSizes() {
        let pieces = ReplaySource.split("abcdefghij", sizes: [1, 3, 2])
        #expect(pieces == ["a", "bcd", "ef", "gh", "ij"])
    }
}

/// Playing a case in the viewer gives the reference's result at every step,
/// for fixed and recorded chunkings: the golden match shows a pass throughout.
@MainActor @Suite struct PlayingAgainstTheReference {
    @Test(arguments: ["hand/04-nested-card-mixed-args", "hand/71-expression-forms", "bench/dashboard"])
    func everyStepMatchesItsGolden(caseName: String) async throws {
        let corpus = try Corpus.load()
        let c = try #require(corpus.cases.first { $0.name == caseName })
        let golden = try #require(corpus.golden(for: c))
        let core = Core()
        let library = try await core.library(schema: corpus.schema(of: c))
        let program = try corpus.program(of: c)
        for chunking in [Chunking.fixed(3), .fixed(16)] + golden.recordedChunkings {
            let playback = Playback(core: core, library: library, root: c.root)
            await playback.load(chunking.source(for: program))
            #expect(playback.chunks.count == golden.steps[chunking.goldenName]?.count, "\(chunking)")
            for k in 1...playback.chunks.count {
                await playback.move(to: k)
                let verdict = golden.verdict(step: k, json: playback.step?.json ?? "", chunking: chunking)
                #expect(verdict == .matches, "\(caseName) \(chunking.label) step \(k)")
                if verdict != .matches { break }
            }
        }
    }
}

@Suite struct TheElementTree {
    @Test func rowsFollowTheDrawingsPaths() async throws {
        let core = Core()
        let library = try await core.library(schema: TheCore.schema)
        let step = try await core.parseOneShot(
            "root = Stack([a, Stack([b])])\na = Text(\"x\")\nb = Text(\"y\")",
            library: library
        )
        let rows = TreeRow.rows(of: try #require(step.result.root))
        #expect(rows.map(\.id) == ["/", "/children/0", "/children/1", "/children/1/children/0"])
        #expect(rows.map(\.depth) == [0, 1, 1, 2])
        #expect(rows.map(\.typeName) == ["Stack", "Text", "Stack", "Text"])
        #expect(rows[1].key == "children[0]")
    }
}

@MainActor @Suite struct NativeDrawing {
    /// Upstream's example library, which the native components follow.
    func bench(_ core: Core) async throws -> Library {
        let corpus = try Corpus.load()
        let path = try #require(corpus.schemas["bench"])
        return try await core.library(schema: String(contentsOf: corpus.directory.appendingPathComponent(path), encoding: .utf8))
    }

    func root(_ program: String) async throws -> Element {
        let core = Core()
        return try #require(try await core.parseOneShot(program, library: bench(core)).result.root)
    }

    /// Every corpus case's final result draws, natively where it can, without
    /// crashing. A root with nothing to draw, such as a stack whose only child
    /// was an unknown component the core dropped, makes an empty image.
    @Test func everyCorpusCaseDrawsOffscreen() async throws {
        let corpus = try Corpus.load()
        let core = Core()
        var libraries: [String: Library] = [:]
        var drawn = 0
        var empty = 0
        for c in corpus.cases {
            let library: Library
            if let loaded = libraries[c.schemaName] {
                library = loaded
            } else {
                library = try await core.library(schema: corpus.schema(of: c))
                libraries[c.schemaName] = library
            }
            let step = try await core.parseOneShot(corpus.program(of: c), library: library, root: c.root)
            guard let root = step.result.root else { continue }
            let renderer = ImageRenderer(content: ElementView(element: root))
            renderer.proposedSize = ProposedViewSize(width: 800, height: nil)
            if renderer.nsImage == nil {
                var size = CGSize.zero
                renderer.render { s, _ in size = s }
                #expect(size.height == 0, "\(c.name) has something to draw but made no image")
                empty += 1
            }
            drawn += 1
        }
        #expect(drawn > 200)
        #expect(empty < 25)
    }

    /// The gallery parses cleanly, and every element in it is either drawn
    /// natively or a child part its parent draws.
    @Test func theGalleryIsAllNative() async throws {
        let core = Core()
        let step = try await core.parseOneShot(Gallery.program, library: bench(core))
        #expect(step.result.meta.errors.isEmpty)
        #expect(step.result.meta.unresolved.isEmpty)
        let root = try #require(step.result.root)
        let childParts: Set = ["Col", "SelectItem", "Series", "Slice", "TabItem"]
        let rows = TreeRow.rows(of: root)
        #expect(Set(rows.map(\.typeName)).isSuperset(of: NativeComponents.names))
        for row in rows where !childParts.contains(row.typeName) {
            #expect(NativeComponents.names.contains(row.typeName), "\(row.typeName) at \(row.id)")
        }
        func check(_ element: Element, _ path: String) {
            if NativeComponents.names.contains(element.typeName) {
                #expect(NativeComponents.view(for: element, path: path) != nil, "\(element.typeName) at \(path) fell back")
            }
        }
        visit(root, path: ElementPath.root, check)
    }

    /// The core drops elements with missing required props, so these are built by hand.
    func element(_ type: String, _ props: [(String, PropValue)] = [], partial: Bool = false) -> Element {
        Element(typeName: type, props: props.map { Prop(name: $0.0, value: $0.1) },
                partial: partial, hasDynamicProps: false, statementId: nil)
    }

    @Test func propsOfTheWrongTypeFallBackToABox() {
        let stack = element("Stack", [("children", .scalar(.string("not a list")))])
        #expect(NativeComponents.view(for: stack, path: "/") == nil)
        let series = element("Series", [("category", .scalar(.string("s"))), ("values", .list([.scalar(.string("x"))]))])
        let chart = element("LineChart", [("labels", .list([.scalar(.string("a"))])), ("series", .list([.element(series)]))])
        #expect(NativeComponents.view(for: chart, path: "/") == nil)
        let goodSeries = element("Series", [("category", .scalar(.string("s"))), ("values", .list([.scalar(.number(1))]))])
        let goodChart = element("LineChart", [("labels", .list([.scalar(.string("a"))])), ("series", .list([.element(goodSeries)]))])
        #expect(NativeComponents.view(for: goodChart, path: "/") != nil)
    }

    @Test func aMissingPropFallsBackOnlyOnceTheElementIsComplete() {
        for type in ["TextContent", "Callout", "Button", "Table"] {
            #expect(NativeComponents.view(for: element(type), path: "/") == nil, "complete \(type)")
            #expect(NativeComponents.view(for: element(type, partial: true), path: "/") != nil, "arriving \(type)")
        }
    }

    @Test func dynamicTextIsDrawnAsItsSource() async throws {
        let text = try await root(#"root = TextContent($name)"#)
        #expect(try PropReader(element: text).text("text") == .dynamic("ƒ " + #"{"k":"StateRef","n":"$name"}"#))
        #expect(NativeComponents.view(for: text, path: "/") != nil)
    }

    private func visit(_ element: Element, path: String, _ body: (Element, String) -> Void) {
        body(element, path)
        func walk(_ value: PropValue, _ path: String) {
            switch value {
            case let .element(e): visit(e, path: path, body)
            case let .list(items): for (i, item) in items.enumerated() { walk(item, ElementPath.child(of: path, String(i))) }
            case let .record(fields): for f in fields { walk(f.value, ElementPath.child(of: path, f.name)) }
            default: break
            }
        }
        for prop in element.props { walk(prop.value, ElementPath.child(of: path, prop.name)) }
    }
}
