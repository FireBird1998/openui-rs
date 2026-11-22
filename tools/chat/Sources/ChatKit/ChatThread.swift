import Foundation
import Observation
import OpenUINative

/// What the template shows and sends, copied from upstream's openui-cloud
/// template and react-ui (#129, amendment 12).
public enum Template {
    /// The default model (#129); the template's own default is
    /// `google/gemini-3.6-flash-free`, which takes over if the Gateway refuses
    /// this one.
    public static let defaultModel = "anthropic/claude-sonnet-5.5"
    public static let welcomeTitle = "What's on your mind today?"
    public static let placeholder = "Ask anything..."
    public static let errorTitle = "Something went wrong"
    /// react-ui's description when an error has no message.
    public static let errorFallback = "An unexpected error occurred. Please try again."
    public static let starters: [(displayText: String, prompt: String)] = [
        ("Compare breakfast nutrition",
         "Compare the nutrition of common breakfast foods: oatmeal, eggs, Greek yogurt, avocado toast, and a banana."),
        ("See how compound interest grows",
         "Explain how compound interest works using $10,000 invested at 7% for 30 years."),
        ("Help me pick a car",
         "Help me pick the right type of car for my lifestyle. Ask me a few questions first."),
    ]
}

/// One conversation with the Gateway, as upstream's chat store runs one:
/// sending appends the person's message and asks for an answer with the
/// whole thread; the answer is drawn as it streams; stopping ends the
/// request and keeps what arrived; a failure shows its message and keeps the
/// person's message.
@MainActor @Observable
public final class ChatThread {
    public enum Turn: Identifiable {
        case user(id: UUID, text: String)
        case assistant(AssistantTurn)

        public var id: UUID {
            switch self {
            case let .user(id, _): id
            case let .assistant(turn): turn.id
            }
        }
    }

    public private(set) var turns: [Turn] = []
    /// The composer's text. It stays editable while an answer runs.
    public var draft = ""
    /// An answer is being asked for; the send button is a stop button.
    public private(set) var isRunning = false
    /// The message under "Something went wrong", or nil when there is none.
    public private(set) var error: String?
    public let model: String

    @ObservationIgnored private let library: ChatLibrary
    @ObservationIgnored private let gateway: any GatewayClient
    @ObservationIgnored private var run: Task<Void, Never>?
    /// For tests: given to each answer as its `refusesDraw`.
    @ObservationIgnored var refusesDraw: (@MainActor (Int) -> OpenUIError?)?

    public init(library: ChatLibrary, gateway: any GatewayClient, model: String = Template.defaultModel) {
        self.library = library
        self.gateway = gateway
        self.model = model
    }

    /// Sends the composer's text, as upstream's composer does: nothing
    /// happens while an answer runs or when the text is blank.
    public func sendDraft() {
        guard send(draft) else { return }
        draft = ""
    }

    /// Sends `text` as the person's next message. Returns whether it was sent.
    @discardableResult
    public func send(_ text: String) -> Bool {
        guard !isRunning, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        error = nil
        turns.append(.user(id: UUID(), text: text))
        isRunning = true
        let request = ChatRequest(model: model, messages: [ChatMessage(.system, library.systemPrompt)] + history)
        run = Task { await answer(request) }
        return true
    }

    /// Ends the running request. What arrived stays on screen, and no error
    /// is shown.
    public func stop() {
        run?.cancel()
    }

    /// Waits for the running answer, if any, to end and be drawn.
    public func waitUntilIdle() async {
        await run?.value
    }

    /// The thread as it is sent: each person's message, and each answer as
    /// the raw text it streamed.
    var history: [ChatMessage] {
        turns.map { turn in
            switch turn {
            case let .user(_, text): ChatMessage(.user, text)
            case let .assistant(answer): ChatMessage(.assistant, answer.rawText)
            }
        }
    }

    /// The answer being received, while a request runs.
    @ObservationIgnored private var current: AssistantTurn?

    private func answer(_ request: ChatRequest) async {
        current = nil
        do {
            try await Self.read(gateway.answer(request)) { [weak self] delta in
                await self?.receive(delta) ?? false
            }
        } catch {
            if !Task.isCancelled && !(error is CancellationError) {
                self.error = Self.message(for: error)
            }
        }
        // A stopped answer is drawn too: in its own task, which the stop's
        // cancellation does not reach.
        if let answer = current { await Task { await answer.finish() }.value }
        current = nil
        isRunning = false
        run = nil
    }

    /// Reads the answer off the main actor and hands each delta on, waiting
    /// for it to be taken before reading the next, so nothing is read ahead
    /// of what the session has accepted. When a delta is refused, reading
    /// stops, which cancels the request.
    private nonisolated static func read(
        _ answer: sending any AsyncSequence<String, any Error>,
        into receive: @escaping @MainActor @Sendable (String) async -> Bool
    ) async throws {
        for try await delta in answer {
            guard await receive(delta) else { return }
        }
    }

    /// Takes one delta into the current answer; false when the answer's
    /// session refused it.
    private func receive(_ delta: String) async -> Bool {
        if current == nil {
            do {
                let turn = try AssistantTurn(library: library)
                turn.refusesDraw = refusesDraw
                turns.append(.assistant(turn))
                current = turn
            } catch {
                self.error = Self.message(for: error)
                return false
            }
        }
        return await current?.receive(delta) ?? false
    }

    static func message(for error: any Error) -> String {
        let message = (error as? GatewayError)?.message ?? (error as? OpenUIError)?.message ?? error.localizedDescription
        return message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? Template.errorFallback : message
    }
}

/// One answer: the raw text it streamed and the live program it is drawn
/// from. The session takes each delta before the next is read, so the text
/// holds only what the session accepted; drawing follows behind, and the
/// deltas that arrive during one draw are drawn together by the next (#206).
@MainActor @Observable
public final class AssistantTurn: Identifiable {
    public let id = UUID()
    /// The answer as the session accepted it.
    public private(set) var rawText = ""
    public let live: LiveProgram
    /// Why the answer could not be drawn, if it could not: the session's own
    /// message, such as its refusal of a response past its program-size
    /// limit.
    public private(set) var drawFailure: String?
    /// How many streaming draws ran, for tests.
    @ObservationIgnored private(set) var draws = 0
    /// For tests: given the number of a streaming draw that succeeded, a
    /// refusal to report in its place, as the session reports one for a
    /// limit it reaches only when drawing.
    @ObservationIgnored var refusesDraw: (@MainActor (Int) -> OpenUIError?)?

    /// Text the session refused for a reason other than one of its limits:
    /// kept, as the app always kept it, and handed over again with the next
    /// delta, so the session reads the whole text. Reading refuses only for
    /// limits today; this keeps the app and the session in step should that
    /// change.
    @ObservationIgnored private var untaken = ""
    /// The draw that is running, if one is.
    @ObservationIgnored private var drawing: Task<Void, Never>?
    /// The session took text that is not drawn yet.
    @ObservationIgnored private var undrawn = false
    /// A draw was refused for one of the session's limits.
    @ObservationIgnored private var refused = false
    /// How much of the text the last streaming draw that succeeded drew.
    @ObservationIgnored private var drawnBytes = 0
    @ObservationIgnored private var finished = false

    init(library: ChatLibrary) throws(OpenUIError) {
        live = try LiveProgram(library: library.library, promptSpec: library.promptSpec)
        live.isStreaming = true
    }

    /// Hands `delta` to the session, to be drawn by the next draw. Returns
    /// false, keeping the text as it was, when the session
    /// refuses it for one of its limits: the response reader refuses a whole
    /// response past its program-size limit before reading any of it. Text
    /// the session read but could not take is kept; the finished draw
    /// reports it.
    func receive(_ delta: String) async -> Bool {
        guard !refused else { return false }
        let piece = untaken + delta
        let refusal: OpenUIError?
        do {
            try await live.append(response: piece, streaming: true)
            refusal = nil
        } catch {
            refusal = error
        }
        // A draw refused while this delta was being taken ended the answer
        // at the text it drew and cut the text back to it: adding the delta
        // now would leave a gap.
        guard !refused else { return false }
        if let refusal {
            if refusal.kind == .limitExceeded {
                drawFailure = refusal.message
                return false
            }
            untaken = piece
        } else {
            untaken = ""
        }
        rawText += delta
        draw()
        return true
    }

    /// Draws what the session has taken, unless drawing is under way: then
    /// the next draw takes in everything taken by the time it starts.
    ///
    /// A draw holds the main actor and the core's thread, which taking a
    /// delta needs too, so while one runs no delta is taken. After each draw
    /// the next waits (`pause(afterDrawThatTook:)`), and the deltas taken
    /// meanwhile are drawn together. A slow answer is drawn delta by delta,
    /// as before; a fast one is drawn less often as it grows, rather than
    /// once per delta at a cost that grows with it.
    private func draw() {
        undrawn = true
        guard drawing == nil else { return }
        drawing = Task {
            while undrawn && !finished {
                undrawn = false
                let started = ContinuousClock.now
                await drawTaken()
                try? await Task.sleep(for: Self.pause(afterDrawThatTook: ContinuousClock.now - started))
            }
            drawing = nil
        }
    }

    /// How long the next streaming draw waits after one: as long as that
    /// draw took. The pause comes from the draws' own cost, not from a
    /// number in the app.
    ///
    /// Why: drawing then gets at most half the time, however long the
    /// answer, so taking deltas, and with it reading, is never starved.
    ///
    /// The review of #206 measured the alternatives against it:
    /// - Drawing again right after a draw, with only a `Task.yield`, is as
    ///   slow as drawing every delta was: 31 KB took 12.6 s, 106 KB 125.8 s.
    /// - Buffering the deltas that arrive during a draw, with no timed wait,
    ///   matches its speed, but stop latency doubles: 206 ms at 106 KB
    ///   against 82–93 ms, and 1.30 s at 1 MiB against 0.65 s.
    /// - That buffered form also reads ahead of what the session has taken:
    ///   18.5 KB ahead at a stop at 71.6 KB, and 1.51 MB read against
    ///   1.29 MB in the endless case, which breaks #150's "nothing read
    ///   ahead".
    private static func pause(afterDrawThatTook drawTook: Duration) -> Duration {
        drawTook
    }

    /// A draw the session refuses for one of its limits ends the answer at
    /// the last text that drew, as a refused delta always did: reading stops,
    /// which cancels the request, and the finished draw shows that text.
    private func drawTaken() async {
        draws += 1
        do throws(OpenUIError) {
            let step = try await live.showResponse(streaming: true)
            if let refusal = refusesDraw?(draws) { throw refusal }
            drawnBytes = step.response?.sourceByteCount ?? drawnBytes
        } catch where error.kind == .limitExceeded {
            drawFailure = error.message
            refused = true
            rawText = String(decoding: rawText.utf8.prefix(drawnBytes), as: UTF8.self)
            untaken = ""
        } catch {}
    }

    /// Draws the answer as complete: after it streamed to the end, stopped,
    /// failed or was refused. It supersedes a streaming draw still running.
    func finish() async {
        finished = true
        live.isStreaming = false
        do {
            try await live.show(response: rawText, streaming: false)
        } catch {
            drawFailure = error.message
        }
    }
}
