import AppKit
import Foundation

/// The conformance-independent request list, also used as a recording's resume
/// identity. Comparing the whole value prevents stale prompts from being skipped.
public struct BatchRequest: Codable, Equatable, Sendable {
    public enum Model: String, Codable, Sendable {
        case sonnet, haiku
        var claude: ClaudeModel { self == .sonnet ? .sonnet : .haiku }
    }
    public var number: Int
    public var model: Model
    public var request: String
    public var of: Int?

    var draft: LiveDraft {
        LiveDraft(text: request, model: model.claude, effort: "low", library: Corpus.preferredLibrary)
    }
}

struct BatchRecordOptions {
    var requests: [BatchRequest]
    var from: Int
    var to: Int

    init(data: Data, from: Int = 1, to: Int = .max) throws {
        requests = try JSONDecoder().decode([BatchRequest].self, from: data)
        self.from = from
        self.to = to
        guard from > 0, to >= from, !requests.isEmpty else {
            throw ClaudeError(description: "Invalid request range or empty list")
        }
        var previous = 0
        var seen: [Int: BatchRequest] = [:]
        for item in requests {
            guard item.number > previous, !item.request.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw ClaudeError(description: "Requests must be nonempty, unique and ordered by number")
            }
            if let of = item.of {
                guard let parent = seen[of], parent.of == nil, parent.model == item.model else {
                    throw ClaudeError(description: "A follow-up must name an earlier fresh request with the same model")
                }
            }
            seen[item.number] = item
            previous = item.number
        }
    }

    init(environment: [String: String]) throws {
        guard let path = environment["VIEWER_RECORD"], !path.isEmpty else {
            throw ClaudeError(description: "VIEWER_RECORD needs a JSON path")
        }
        func bound(_ key: String, default value: Int) throws -> Int {
            guard let text = environment[key] else { return value }
            guard let number = Int(text), number > 0 else {
                throw ClaudeError(description: "Invalid batch range")
            }
            return number
        }
        try self.init(data: Data(contentsOf: URL(fileURLWithPath: path)),
                      from: bound("VIEWER_RECORD_FROM", default: 1),
                      to: bound("VIEWER_RECORD_TO", default: .max))
    }

    var selected: [BatchRequest] { requests.filter { (from...to).contains($0.number) } }
}

extension ClaudeRequest {
    /// A deliberately high estimate, without a token-count API call: one token
    /// per UTF-8 byte plus envelope headroom, cache-write overhead, and all
    /// allowed output tokens. Fallbacks reserve at the highest viewer price.
    /// This is a local pricing bound, not a guarantee of the provider's bill.
    var budgetReservation: Double {
        let models = model.fallbacks ? ClaudeModel.all : [model]
        let inputPrice = models.map(\.inputPrice).max() ?? model.inputPrice
        let outputPrice = models.map(\.outputPrice).max() ?? model.outputPrice
        return (Double(systemPrompt.utf8.count + userText.utf8.count + 1024) * inputPrice * 1.25
                + Double(maxTokens) * outputPrice) / 1_000_000
    }
}

/// Injected only in tests; production uses the viewer's existing preferences.
@MainActor
struct BatchBudget {
    var cap: () -> Double
    var spent: () -> Double
    var add: (Double) -> Void

    static var viewer: Self { Self(cap: { Budget.cap }, spent: { Budget.spent }, add: { Budget.add($0) }) }

    func permits(_ reservation: Double) -> Bool {
        cap().isFinite && spent().isFinite && spent() >= 0 && spent() + reservation <= cap()
    }
}

@MainActor
final class BatchRecorder {
    struct Summary {
        var recorded = 0
        var skipped = 0
        var failed = 0
        var spend = 0.0
        var line: String {
            "batch summary: recorded=\(recorded) skipped=\(skipped) failed=\(failed) estimated spend=\(Budget.format(spend))"
        }
    }

    typealias SourceFactory = @MainActor (ClaudeRequest, LiveStatus) throws -> any ChunkSource
    let model: CorpusModel
    let directory: URL
    let budget: BatchBudget
    let source: SourceFactory
    let output: (String) -> Void

    init(model: CorpusModel, directory: URL = Recordings.directory, budget: BatchBudget = .viewer,
         source: SourceFactory? = nil,
         output: @escaping (String) -> Void = { print($0); fflush(stdout) }) {
        self.model = model
        self.directory = directory
        self.budget = budget
        self.source = source ?? CorpusModel.batchSource()
        self.output = output
    }

    func run(_ options: BatchRecordOptions) async -> Summary {
        var summary = Summary()
        var consecutiveFailures = 0
        var stopped: String?
        let saved = Recordings.list(in: directory)
        var programs: [Int: String] = [:]

        func matching(_ item: BatchRequest) -> Recording? {
            saved.first {
                let r = $0.recording
                return r.batchRequest == item && r.request == item.request && r.model == item.model.claude.id
                    && r.library == Corpus.preferredLibrary && r.effort == "low" && !r.text.isEmpty
                    && (r.stopReason == "end_turn" || r.stopReason == "max_tokens")
                    && (r.patchOf == nil) == (item.of == nil)
            }?.recording
        }
        func line(_ item: BatchRequest, _ outcome: String, _ reason: String, status: LiveStatus? = nil, cost: Double = 0) {
            // Reasons are fixed strings, never server bodies, headers, keys or
            // arbitrary error descriptions. Keep exactly one line per request.
            output("\(item.number) \(item.model.rawValue) \(outcome): \(reason)"
                   + " | tokens=\(status?.inputTokens ?? 0) in/\(status?.outputTokens ?? 0) out estimated=\(Budget.format(cost))")
        }
        func failed() {
            summary.failed += 1
            consecutiveFailures += 1
            if consecutiveFailures >= 3 { stopped = "stopped after 3 consecutive failures" }
        }

        for item in options.selected {
            if Task.isCancelled { stopped = "batch cancelled" }
            if let stopped {
                summary.skipped += 1
                line(item, "skipped", stopped)
                continue
            }
            if let recording = matching(item) {
                if item.of == nil { programs[item.number] = recording.text }
                summary.skipped += 1
                line(item, "skipped", "already recorded")
                continue
            }
            var draft = item.draft
            if let parentNumber = item.of {
                if programs[parentNumber] == nil,
                   let parent = options.requests.first(where: { $0.number == parentNumber }),
                   let recording = matching(parent) {
                    programs[parentNumber] = recording.text
                }
                guard let base = programs[parentNumber] else {
                    summary.skipped += 1
                    line(item, "skipped", "fresh request \(parentNumber) has no available recording")
                    continue
                }
                draft.base = base
            }
            let prepared: CorpusModel.PreparedLive
            do { prepared = try await model.prepareLive(draft) }
            catch {
                failed()
                line(item, "failed", "could not prepare library or prompt")
                continue
            }
            let reservation = prepared.request.budgetReservation
            guard budget.permits(reservation) else {
                stopped = "budget cap would be exceeded (request reserve \(Budget.format(reservation)))"
                summary.skipped += 1
                line(item, "skipped", stopped!)
                continue
            }
            let status = LiveStatus(model: draft.model)
            let stream: any ChunkSource
            do { stream = try source(prepared.request, status) }
            catch {
                failed()
                line(item, "failed", "could not create stream; check the Keychain key in Settings")
                continue
            }
            // Persist before sending, so interrupting the app cannot erase the
            // in-flight request's budget. Unknown usage keeps this reservation.
            budget.add(reservation)
            await model.streamLive(prepared, status: status, source: stream)
            let cost = charge(status, reservation: reservation)
            budget.add(cost - reservation)
            summary.spend += cost

            let complete: Bool
            switch status.phase {
            case .finished("end_turn"), .finished("max_tokens"): complete = !status.received.isEmpty
            default: complete = false
            }
            // Keep even partial streams for investigation, but only completed
            // streams get a resume identity. A later run retries failures.
            do {
                if !status.received.isEmpty {
                    try model.saveRecording(batchRequest: complete ? item : nil, in: directory)
                }
            } catch {
                failed()
                line(item, "failed", "could not save recording", status: status, cost: cost)
                continue
            }
            guard complete else {
                failed()
                let reason: String
                switch status.phase {
                case .finished("refusal"): reason = "model refused request"
                case .cancelled: reason = "stream cancelled; reservation retained if usage unknown"
                // Only the status number: the API's message is untrusted text.
                case .failed(let message) where message.hasPrefix("HTTP "):
                    let code = message.dropFirst(5).prefix(while: \.isNumber)
                    reason = "stream failed with HTTP \(code); reservation retained if usage unknown"
                default: reason = "stream failed or incomplete; reservation retained if usage unknown"
                }
                line(item, "failed", reason, status: status, cost: cost)
                continue
            }
            summary.recorded += 1
            consecutiveFailures = 0
            if item.of == nil { programs[item.number] = model.current?.finalProgram ?? status.received.map(\.text).joined() }
            let reason = status.phase == .finished(stopReason: "max_tokens") ? "saved (output token limit)" : "saved"
            line(item, "ok", reason, status: status, cost: cost)
        }
        output(summary.line)
        return summary
    }

    private func charge(_ status: LiveStatus, reservation: Double) -> Double {
        guard status.hasInputUsage, status.hasOutputUsage,
              case let .finished(reason) = status.phase,
              ["end_turn", "max_tokens", "refusal"].contains(reason),
              let served = ClaudeModel.all.first(where: { $0.id == status.servedBy }) else { return reservation }
        // Cache reads may be cheaper; treating all input as cache writes errs
        // high. Keep at least the requested model's prices after a fallback.
        return (Double(status.inputTokens) * max(served.inputPrice, status.model.inputPrice) * 1.25
                + Double(status.outputTokens) * max(served.outputPrice, status.model.outputPrice)) / 1_000_000
    }
}

/// Like VIEWER_PLAY, runs once inside the real app and quits when finished.
@MainActor
enum BatchRecordRun {
    static func run(model: CorpusModel) async {
        do {
            let options = try BatchRecordOptions(environment: ProcessInfo.processInfo.environment)
            _ = await BatchRecorder(model: model).run(options)
        } catch {
            print("batch failed: invalid or unreadable VIEWER_RECORD JSON/range")
            print("batch summary: recorded=0 skipped=0 failed=1 estimated spend=$0.0000")
        }
        fflush(stdout)
        NSApp.terminate(nil)
    }
}
