import Foundation
import Testing
@testable import ViewerKit

/// No Keychain or URLSession: drives the same LiveStatus and Playback used by
/// ClaudeSource, yielding deltas before reporting the terminal event.
private struct FakeBatchStream: ChunkSource {
    let status: LiveStatus
    var text = "root = TextContent(\"hello\")"
    var ending = "end_turn"
    var fails = false
    var servedModel: String?
    var onStart: @MainActor @Sendable () -> Void = {}

    func chunks() -> AsyncThrowingStream<[String], any Error> {
        AsyncThrowingStream { continuation in
            let task = Task { @MainActor in
                onStart()
                status.apply(.started(model: servedModel ?? status.model.id, inputTokens: 100))
                for (index, piece) in ReplaySource.split(text, size: 8).enumerated() {
                    status.apply(.text(piece))
                    status.receive(piece, after: .milliseconds(index * 10))
                    continuation.yield([piece])
                    await Task.yield()
                }
                if fails {
                    // This untrusted error must never appear in batch stdout.
                    status.apply(.failed("untrusted server body\nextra log line"))
                    continuation.finish(throwing: ClaudeError(description: "untrusted server body"))
                } else {
                    status.apply(.progress(outputTokens: 20, stopReason: ending == "EOF" ? nil : ending))
                    status.end(nil)
                    continuation.finish()
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

@MainActor
private final class BatchHarness {
    let directory: URL
    let model: CorpusModel
    var cap = 10.0
    var spent = 0.0
    var lines: [String] = []
    var sent: [ClaudeRequest] = []
    var stream: (ClaudeRequest, LiveStatus) throws -> any ChunkSource = { _, status in FakeBatchStream(status: status) }

    init() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        model = CorpusModel(corpus: try Corpus.load())
    }

    func run(_ requests: [BatchRequest], from: Int = 1, to: Int = .max) async throws -> BatchRecorder.Summary {
        let options = try BatchRecordOptions(data: JSONEncoder().encode(requests), from: from, to: to)
        let recorder = BatchRecorder(model: model, directory: directory,
                                     budget: BatchBudget(cap: { self.cap }, spent: { self.spent }, add: { self.spent += $0 }),
                                     source: { request, status in
                                         self.sent.append(request)
                                         return try self.stream(request, status)
                                     }, output: { self.lines.append($0) })
        return await recorder.run(options)
    }

    func clean() { try? FileManager.default.removeItem(at: directory) }
}

@MainActor @Suite struct RecordingABatch {
    private func request(_ n: Int, of: Int? = nil, text: String? = nil) -> BatchRequest {
        BatchRequest(number: n, model: n % 2 == 1 ? .sonnet : .haiku, request: text ?? "screen \(n)", of: of)
    }

    @Test func streamsSequentiallyWithTheSheetsPromptAndSavesNormalRecordings() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        h.stream = { request, status in
            if request.model == .haiku {
                // The first stream must already be saved before this starts.
                #expect(Recordings.list(in: h.directory).count == 1)
            }
            return FakeBatchStream(status: status)
        }
        let result = try await h.run([request(1), request(2)])
        #expect(result.recorded == 2 && result.failed == 0 && result.skipped == 0)
        #expect(h.sent.map(\.model.id) == [ClaudeModel.sonnet.id, ClaudeModel.haiku.id])
        #expect(h.sent.allSatisfy { $0.effort == "low" && $0.maxTokens == 16_000 })
        let sheet = try await h.model.prepareLive(request(1).draft)
        #expect(h.sent[0].systemPrompt == sheet.request.systemPrompt)
        #expect(h.sent[0].userText == "screen 1")
        let saved = Recordings.list(in: h.directory)
        #expect(saved.count == 2)
        #expect(saved.allSatisfy { $0.recording.library == "react-ui" && $0.recording.batchRequest != nil })
        #expect(saved.allSatisfy { $0.recording.chunks.count > 1 && $0.recording.outputTokens == 20 })
        #expect(abs(result.spend - h.spent) < 1e-10)
        #expect(h.lines.count == 3)
        #expect(h.lines[0].hasPrefix("1 sonnet ok: saved | tokens=100 in/20 out estimated=$"))
        #expect(h.lines[1].hasPrefix("2 haiku ok:"))
        #expect(h.lines[2].contains("recorded=2 skipped=0 failed=0 estimated spend="))
        #expect(h.lines.allSatisfy { !$0.contains("\n") })
    }

    @Test func resumeSkipsCompletedRequestsAndResolvesAnEarlierRunsParentOutsideRange() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        let requests = [request(1), request(2), request(3, of: 1, text: "make it blue")]
        _ = try await h.run(requests, to: 1)
        let original = try #require(Recordings.list(in: h.directory).first?.recording)
        h.sent = []
        h.lines = []
        let resumed = try await h.run(requests, from: 3)
        #expect(resumed.recorded == 1)
        #expect(h.sent.count == 1)
        #expect(h.sent[0].userText == ClaudeRequest.followUp("make it blue", to: original.text))
        #expect(h.sent[0].systemPrompt.contains("## Edit Mode"))
        let followUp = try #require(Recordings.list(in: h.directory).first { $0.recording.batchRequest?.number == 3 })
        #expect(followUp.recording.patchOf == original.text)
        h.sent = []
        h.lines = []
        let again = try await h.run(requests)
        #expect(again.recorded == 1 && again.skipped == 2)
        #expect(h.sent.map(\.userText) == ["screen 2"])
        #expect(h.lines[0].contains("skipped: already recorded"))
        #expect(h.lines[2].contains("skipped: already recorded"))
    }

    @Test func followUpsUseTheirOwnFreshProgramNotTheLastScreenOrAnotherPatch() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        h.stream = { req, status in
            FakeBatchStream(status: status, text: req.userText.hasPrefix("Here is")
                            ? "root = TextContent(\"changed\")" : "root = TextContent(\"\(req.userText)\")")
        }
        let result = try await h.run([request(1), request(2), request(3, of: 1), request(5, of: 1)])
        #expect(result.recorded == 4)
        let base = "root = TextContent(\"screen 1\")"
        #expect(h.sent[2].userText == ClaudeRequest.followUp("screen 3", to: base))
        #expect(h.sent[3].userText == ClaudeRequest.followUp("screen 5", to: base))
        #expect(h.model.current?.finalProgram?.contains("changed") == true)
    }

    @Test func missingParentsAreSkippedAndChangedRequestsAreNotMistakenForSavedOnes() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        let requests = [request(1), request(3, of: 1)]
        let missing = try await h.run(requests, from: 3)
        #expect(missing.skipped == 1 && h.sent.isEmpty)
        #expect(h.lines[0].contains("fresh request 1 has no available recording"))
        _ = try await h.run(requests, to: 1)
        h.sent = []
        let changed = [request(1, text: "changed request"), request(3, of: 1)]
        let unavailable = try await h.run(changed, from: 3)
        #expect(unavailable.skipped == 1 && h.sent.isEmpty)
        let rerun = try await h.run(changed, to: 1)
        #expect(rerun.recorded == 1 && h.sent.count == 1)
    }

    @Test func budgetStopsBeforeCreatingAStreamAndIncludesTheFollowUpBase() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        let prepared = try await h.model.prepareLive(request(1).draft)
        h.cap = prepared.request.budgetReservation - 0.00001
        let result = try await h.run([request(1), request(2)])
        #expect(result.skipped == 2 && result.spend == 0 && h.sent.isEmpty)
        #expect(h.lines[0].contains("budget cap would be exceeded"))
        #expect(h.lines[1].contains("budget cap would be exceeded"))
        var draft = request(3, of: 1).draft
        draft.base = String(repeating: "long program", count: 1000)
        let edit = try await h.model.prepareLive(draft)
        #expect(edit.request.budgetReservation > prepared.request.budgetReservation)
    }

    @Test func budgetReservesBeforeSendingAndAccountsForEarlierSpend() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        let prepared = try await h.model.prepareLive(request(1).draft)
        h.spent = 2
        h.cap = h.spent + prepared.request.budgetReservation
        h.stream = { _, status in
            FakeBatchStream(status: status, servedModel: ClaudeModel.opus.id, onStart: {
                #expect(abs(h.spent - h.cap) < 1e-10)
            })
        }
        let result = try await h.run([request(1)])
        #expect(result.recorded == 1)
        let expected = (100.0 * ClaudeModel.opus.inputPrice * 1.25 + 20 * ClaudeModel.opus.outputPrice) / 1_000_000
        #expect(abs(result.spend - expected) < 1e-10)
        #expect(abs(h.spent - 2 - expected) < 1e-10)
    }

    @Test func threeFailuresStopTheRunAndPartialStreamsAreRetriedWithoutLoggingErrors() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        h.stream = { _, status in FakeBatchStream(status: status, fails: true) }
        let requests = (1...5).map { request($0) }
        let result = try await h.run(requests)
        #expect(result.failed == 3 && result.skipped == 2 && h.sent.count == 3)
        #expect(result.spend > 0 && result.spend == h.spent)
        #expect(h.lines.count == 6)
        #expect(h.lines[3].contains("stopped after 3 consecutive failures"))
        #expect(h.lines.allSatisfy { !$0.contains("untrusted") && !$0.contains("\n") })
        let partials = Recordings.list(in: h.directory)
        #expect(partials.count == 3 && partials.allSatisfy { $0.recording.batchRequest == nil })
        h.sent = []
        h.stream = { _, status in FakeBatchStream(status: status) }
        let retry = try await h.run(requests, to: 1)
        #expect(retry.recorded == 1 && h.sent.count == 1)
    }

    @Test func successResetsConsecutiveFailures() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        var call = 0
        h.stream = { _, status in
            call += 1
            return FakeBatchStream(status: status, fails: call != 3)
        }
        let result = try await h.run((1...7).map { request($0) })
        #expect(result.failed == 5 && result.recorded == 1 && result.skipped == 1)
        #expect(h.sent.count == 6)
    }

    @Test func missingParentSkipsDoNotResetConsecutiveFailures() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        h.stream = { _, status in FakeBatchStream(status: status, fails: true) }
        let result = try await h.run([request(1), request(2), request(3, of: 1), request(4), request(5)])
        #expect(result.failed == 3 && result.skipped == 2 && h.sent.count == 3)
        #expect(h.lines[2].contains("no available recording"))
        #expect(h.lines[4].contains("stopped after 3 consecutive failures"))
    }

    @Test func refusalAndPrematureEOFCountAsFailuresAndTokenLimitIsSaved() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        h.stream = { req, status in
            switch req.userText {
            case "screen 1": FakeBatchStream(status: status, text: "", ending: "refusal")
            case "screen 2": FakeBatchStream(status: status, ending: "EOF")
            default: FakeBatchStream(status: status, ending: "max_tokens")
            }
        }
        let result = try await h.run((1...3).map { request($0) })
        #expect(result.failed == 2 && result.recorded == 1)
        #expect(h.lines[0].contains("model refused"))
        #expect(h.lines[2].contains("saved (output token limit)"))
        #expect(Recordings.list(in: h.directory).filter { $0.recording.batchRequest != nil }.count == 1)
    }

    @Test func failedSaveIsCountedAndNotResumed() async throws {
        let h = try BatchHarness()
        defer { h.clean() }
        // A file at the recordings-directory path makes saving fail.
        try Data("file".utf8).write(to: h.directory)
        let result = try await h.run([request(1)])
        #expect(result.failed == 1 && result.recorded == 0 && result.spend > 0)
        #expect(h.lines[0].contains("could not save recording"))
    }

    @Test func invalidListsAndRangesFailBeforeRunning() throws {
        func options(_ rows: [BatchRequest]) throws -> BatchRecordOptions {
            try BatchRecordOptions(data: JSONEncoder().encode(rows))
        }
        #expect(throws: (any Error).self) { try options([request(1), request(1)]) }
        #expect(throws: (any Error).self) { try options([request(2), request(1)]) }
        #expect(throws: (any Error).self) { try options([request(1), request(2, of: 1)]) }
        #expect(throws: (any Error).self) { try options([request(1), request(3, of: 1), request(5, of: 3)]) }
        #expect(throws: (any Error).self) { try options([request(3, of: 1)]) }
        #expect(throws: (any Error).self) { try options([request(1, text: " ")]) }
        #expect(throws: (any Error).self) { try BatchRecordOptions(data: JSONEncoder().encode([request(1)]), from: 4, to: 2) }
        #expect(throws: (any Error).self) {
            try BatchRecordOptions(data: Data(#"[{"number":1,"model":"opus","request":"x","of":null}]"#.utf8))
        }
    }

    @Test func optionalMetadataRoundTripsAndOldRecordingsStillDecode() throws {
        let old = try Recording.decode(handMade.encoded())
        #expect(old.batchRequest == nil)
        var batch = old
        batch.batchRequest = request(1)
        let roundTrip = try Recording.decode(batch.encoded())
        #expect(roundTrip == batch)
        #expect(roundTrip.version == 1 && roundTrip.format == Recording.formatName)
        #expect(roundTrip.chunks == old.chunks && roundTrip.request == old.request)
    }
}
