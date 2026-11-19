import OpenUINative
import Testing
@testable import ViewerKit

@Suite(.timeLimit(.minutes(1))) @MainActor struct ResponsePlayback {
    @Test(arguments: [false, true])
    func openingASamplePlaysItsRawMarkersAndDrawsItsEdition(bundle: Bool) async throws {
        let model = CorpusModel(corpus: try Corpus.load())
        let request = try ProgramRequest.responseSample(bundle: bundle)
        await model.open(request)
        let playback = try #require(model.playback)
        let live = try #require(playback.live)
        let title = bundle ? "Evening edition" : "Morning edition"
        // The bundled provider returns immediately. Only executor hops are awaited.
        for _ in 0..<20_000 {
            if !live.isQueryLoading { break }
            await Task.yield()
        }
        #expect(playback.text == request.text)
        #expect(live.response?.name == title)
        #expect(live.response?.attributes["library"] == "bench")
        #expect(live.hasToolProvider == bundle)
        #expect(live.response?.context == (bundle ? JSONValue.null : .object([("desk", .string("City"))])))
        #expect(live.queries.values["edition"] == (bundle ? .string(title) : nil))
        guard case let .scalar(.string(label))? = playback.drawnRoot?.props.first?.value else {
            Issue.record("Expected a drawn edition"); return
        }
        #expect(label == title)
        let program = try #require(model.current?.finalProgram)
        #expect(!program.contains("]]>") && program.contains("root = TextContent("))
        #expect(playback.step?.program == program)
        let last = playback.position
        await playback.move(to: last - 1)
        #expect(live.isStreaming && !live.acceptsInteractions)
        #expect(live.response?.name == title && playback.step?.program == program)
        await playback.move(to: last)
        #expect(playback.fromCache && !live.isStreaming)
        #expect(live.response?.name == title && playback.step?.program == program)
    }
}
