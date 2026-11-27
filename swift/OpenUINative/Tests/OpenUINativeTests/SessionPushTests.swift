@testable import OpenUINative
import Testing

@Suite @MainActor struct SessionPushTests {
    @Test func lineBoundaryMetadataPreservesPushResultsAndPreviewAuthority() async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let stream = try await core.session(library: library)
        let original = try await core.session(library: library)
        let program = try LiveProgram(library: library)
        let pushes: [([String], Bool)] = [
            ([], false),
            (["root = TextContent(\"hel"], false),
            (["lo\")", "\n"], true),
            ([], true),
            ([""], true),
            ([" ", "\t", ""], true),
            (["extra = 3"], false),
            (["\n"], true),
        ]
        for (chunks, lineBoundary) in pushes {
            let step = try await stream.push(chunks)
            // Exercise the same pre-Image FFI entry points on a second stream.
            let preImage = try await core.run {
                try Step.measure(chunks: chunks.count, streaming: true) {
                    if let chunk = chunks.first, chunks.count == 1 { return original.inner.push(chunk: chunk) }
                    return original.inner.pushAll(chunks: chunks)
                }
            }
            #expect(step.json == preImage.json)
            #expect(step.result.depthLimited == preImage.result.depthLimited)
            #expect(step.result.sizeLimited == preImage.result.sizeLimited)
            #expect(step.timings.chunks == chunks.count)
            #expect(step.program == nil && step.response == nil)
            #expect(step.isStreaming)
            #expect(step.previewEndsAtLineBreak == lineBoundary)
            try program.show(step)
            #expect(program.isStreaming)
        }
        let step = try await stream.finish()
        #expect(step.json == (try await original.finish()).json)
        #expect(!step.isStreaming && step.timings.chunks == 0)
        try program.show(step)
        #expect(!program.isStreaming)
        #expect(texts(drawing: program) == ["/": "hello"])
    }
}
