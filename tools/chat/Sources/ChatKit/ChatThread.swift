import AppKit
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
/// request and keeps what arrived. Failed actions roll back. In a stored
/// thread, every turn remains provisional until the save is confirmed.
@MainActor @Observable
public final class ChatThread {
    public enum Turn: Identifiable {
        case user(UserTurn)
        case assistant(AssistantTurn)

        @MainActor var message: ChatMessage {
            switch self {
            case let .user(turn): ChatMessage(.user, turn.rawText)
            case let .assistant(turn): ChatMessage(.assistant, turn.rawText)
            }
        }

        public var id: UUID {
            switch self {
            case let .user(turn): turn.id
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
    @ObservationIgnored private let openBrowser: @MainActor (URL) -> Void
    @ObservationIgnored private var run: Task<Void, Never>?
    @ObservationIgnored private var stopRequested = false
    @ObservationIgnored var conversation: ConversationLink?
    /// The prefix of `turns` eligible for replay. The visible suffix belongs
    /// to the running request and is discarded unless it commits.
    @ObservationIgnored private var committedCount = 0
    /// For tests: given to each answer as its `refusesDraw`.
    @ObservationIgnored var refusesDraw: (@MainActor (Int) -> OpenUIError?)?

    public init(library: ChatLibrary, gateway: any GatewayClient, model: String = Template.defaultModel,
                openBrowser: @escaping @MainActor (URL) -> Void = { NSWorkspace.shared.open($0) }) {
        self.library = library
        self.gateway = gateway
        self.model = model
        self.openBrowser = openBrowser
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
        startAnswer(isAction: false) { UserTurn(rawText: text, text: text) }
        return true
    }

    private func handle(_ event: HostEvent) {
        if event.type == "open_url" {
            if let address = event.params["url"]?.string, let url = URL(string: address),
               let scheme = url.scheme?.lowercased(), scheme == "https" || scheme == "http",
               let host = url.host(), !host.isEmpty {
                openBrowser(url)
            }
            return
        }
        guard event.type == "continue_conversation", !isRunning else { return }
        startAnswer(isAction: true) {
            let core = self.library.core
            let raw = try await core.buildActionUserMessage(event)
            // A presentation refusal still leaves an already encoded message to send.
            let parsed = try? await core.parseMessage(raw)
            return UserTurn(rawText: raw, text: event.humanFriendlyMessage, parsed: parsed)
        }
    }

    /// Reserve the request before an action message's asynchronous encoding,
    /// so another action or composer send cannot start a competing turn.
    private func startAnswer(isAction: Bool, preparing prepare: @escaping @MainActor () async throws -> UserTurn) {
        error = nil
        isRunning = true
        stopRequested = false
        run = Task { await answer(isAction: isAction, preparing: prepare) }
    }

    private func answer(isAction: Bool, preparing prepare: @MainActor () async throws -> UserTurn) async {
        let conversation = self.conversation
        let requiresCompleteReply = isAction || conversation != nil
        var requestFailure: String?
        var credential: ConversationCredential?
        var user: UserTurn?
        defer {
            if turns.count > committedCount { turns.removeLast(turns.count - committedCount) }
            current = nil
            isRunning = false
            run = nil
        }
        do {
            if let conversation {
                let validated = try await conversation.credential()
                credential = validated
                try await conversation.reconcile(credential: validated, apply: confirmStored)
            }
            try Task.checkCancellation()
            let next = try await prepare()
            try Task.checkCancellation()
            user = next
            turns.append(.user(next))
            let request = ChatRequest(model: model, messages: [ChatMessage(.system, library.systemPrompt)]
                                      + history + [ChatMessage(.user, next.rawText)])
            let receive: @MainActor @Sendable (String) async throws -> Void = { delta in
                try await self.receive(delta, requiresCompleteReply: requiresCompleteReply)
            }
            if let conversation, let credential {
                try await Self.read(conversation.answer(request, credential: credential), into: receive)
            } else {
                try await Self.read(gateway.answer(request), into: receive)
            }
        } catch where requiresCompleteReply && !stopRequested {
            requestFailure = current?.drawFailure ?? Self.message(for: error)
        } catch where !requiresCompleteReply && !Task.isCancelled && !(error is CancellationError) {
            if !(error is OpenUIError) || current?.drawFailure == nil {
                requestFailure = Self.message(for: error)
            }
        } catch {}

        // A stopped answer is drawn too, in a task the stop doesn't cancel.
        if let answer = current { await Task { await answer.finish() }.value }
        let failure = requestFailure ?? (conversation == nil && stopRequested ? nil : current?.drawFailure)
        guard let conversation else {
            if isAction, let failure {
                error = failure
            } else {
                committedCount = turns.count
                error = requestFailure
            }
            return
        }
        if let failure {
            error = failure
            return
        }
        guard let credential, let user else { return }
        // A stop before the first accepted delta leaves no turn to save.
        guard !stopRequested || current?.rawText.isEmpty == false else { return }
        if current?.rawText.isEmpty == true { turns.removeLast() }
        let messages = turns.dropFirst(committedCount).map(\.message)
        do {
            if try await conversation.save(messages, title: user.text, credential: credential,
                                           applyStored: confirmStored) {
                committedCount = turns.count
            }
        } catch {
            self.error = "Could not save this turn. \(Self.message(for: error))"
        }
    }

    /// Ends the running request. What arrived stays on screen, and no error
    /// is shown.
    public func stop() {
        stopRequested = true
        run?.cancel()
    }

    /// Waits for the running answer, if any, to end and be drawn.
    public func waitUntilIdle() async {
        await run?.value
    }

    /// Committed raw message text, including any action markers.
    var history: [ChatMessage] { turns.prefix(committedCount).map(\.message) }

    /// Stage admission of every stored answer before replacing the transcript.
    func restore(_ messages: [ChatMessage]) async throws {
        var restored: [Turn] = []
        for message in messages {
            try Task.checkCancellation()
            switch message.role {
            case .user:
                let parsed = try? await library.core.parseMessage(message.content)
                restored.append(.user(UserTurn(rawText: message.content, text: message.content, parsed: parsed)))
            case .assistant:
                let answer = try makeAssistant(requiresCompleteReply: false)
                try await answer.restore(message.content)
                restored.append(.assistant(answer))
            case .system:
                continue
            }
        }
        turns = restored
        committedCount = turns.count
    }

    private func confirmStored(_ messages: [ChatMessage]) async throws -> Bool {
        let proposed = turns.map(\.message)
        if Self.sameMessages(messages, proposed) {
            committedCount = turns.count
            return true
        }
        if Self.sameMessages(messages, history) {
            turns.removeLast(turns.count - committedCount)
        } else {
            try await restore(messages)
        }
        return false
    }

    private static func sameMessages(_ lhs: [ChatMessage], _ rhs: [ChatMessage]) -> Bool {
        lhs.count == rhs.count && zip(lhs, rhs).allSatisfy {
            $0.role == $1.role && $0.content.utf8.elementsEqual($1.content.utf8)
        }
    }

    /// The answer being received, while a request runs.
    @ObservationIgnored private var current: AssistantTurn?

    /// Reads the answer off the main actor and hands each delta on, waiting
    /// for it to be taken before reading the next, so nothing is read ahead
    /// of what the session has accepted. When a delta is refused, reading
    /// stops, which cancels the request.
    private nonisolated static func read(
        _ answer: sending any AsyncSequence<String, any Error>,
        into receive: @escaping @MainActor @Sendable (String) async throws -> Void
    ) async throws {
        for try await delta in answer {
            try Task.checkCancellation()
            try await receive(delta)
        }
    }

    private func receive(_ delta: String, requiresCompleteReply: Bool) async throws {
        if let current {
            try await current.receive(delta)
            return
        }
        let turn = try makeAssistant(requiresCompleteReply: requiresCompleteReply)
        turn.refusesDraw = refusesDraw
        if requiresCompleteReply {
            // A draw can fail while the reader is waiting for another event.
            turn.onDrawRefusal = { [weak self] in self?.run?.cancel() }
        }
        turns.append(.assistant(turn))
        current = turn
        try await turn.receive(delta)
    }

    private func makeAssistant(requiresCompleteReply: Bool) throws -> AssistantTurn {
        let turn = try AssistantTurn(library: library, requiresCompleteReply: requiresCompleteReply)
        turn.live.onHostEvent = { [weak self] event in self?.handle(event) }
        return turn
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
    @ObservationIgnored private var drawRefusal: OpenUIError?
    @ObservationIgnored var onDrawRefusal: (@MainActor () -> Void)?
    @ObservationIgnored private let requiresCompleteReply: Bool
    /// How much of the text the last streaming draw that succeeded drew.
    @ObservationIgnored private var drawnBytes = 0
    @ObservationIgnored private var finished = false

    init(library: ChatLibrary, requiresCompleteReply: Bool) throws(OpenUIError) {
        self.requiresCompleteReply = requiresCompleteReply
        live = try LiveProgram(library: library.library, promptSpec: library.promptSpec)
        live.isStreaming = true
    }

    func restore(_ text: String) async throws(OpenUIError) {
        try await live.show(response: text, streaming: false)
        live.isStreaming = false
        finished = true
        rawText = text
    }

    /// Hands `delta` to the session, to be drawn by the next draw. A limit
    /// refusal keeps the text as it was. Other refusals may be retried with
    /// the next local composer delta. A stored turn or an action requires
    /// every delta to be accepted.
    func receive(_ delta: String) async throws(OpenUIError) {
        if let drawRefusal { throw drawRefusal }
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
        if let drawRefusal { throw drawRefusal }
        if let refusal {
            if refusal.kind == .limitExceeded || requiresCompleteReply {
                drawFailure = refusal.message
                throw refusal
            }
            untaken = piece
        } else {
            untaken = ""
        }
        rawText += delta
        draw()
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
            while undrawn && !finished && drawRefusal == nil {
                undrawn = false
                let started = ContinuousClock.now
                await drawTaken()
                if finished { break }
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
        } catch where error.kind == .limitExceeded || requiresCompleteReply {
            drawFailure = error.message
            drawRefusal = error
            rawText = String(decoding: rawText.utf8.prefix(drawnBytes), as: UTF8.self)
            untaken = ""
            onDrawRefusal?()
        } catch {}
    }

    /// Draws the answer as complete: after it streamed to the end, stopped,
    /// failed or was refused. A stored turn or an action settles its pending
    /// streaming draw first so any refusal is accounted for before commit.
    func finish() async {
        finished = true
        if requiresCompleteReply { await drawing?.value }
        live.isStreaming = false
        do {
            try await live.show(response: rawText, streaming: false)
        } catch {
            if drawFailure == nil { drawFailure = error.message }
        }
    }
}
