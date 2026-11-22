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
/// from. Each delta is drawn before the next is taken, and the text holds
/// only what the session accepted.
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

    init(library: ChatLibrary) throws(OpenUIError) {
        live = try LiveProgram(library: library.library, promptSpec: library.promptSpec)
        live.isStreaming = true
    }

    /// Draws the answer with `delta` added. Returns false, keeping the text
    /// as it was, when the session refuses it for one of its limits: the
    /// response reader refuses a whole response past its program-size limit
    /// before reading any of it. A preview the session read but could not
    /// draw keeps the text; the finished draw reports it.
    func receive(_ delta: String) async -> Bool {
        let text = rawText + delta
        do {
            try await live.show(response: text, streaming: true)
        } catch where error.kind == .limitExceeded {
            drawFailure = error.message
            return false
        } catch {}
        rawText = text
        return true
    }

    /// Draws the answer as complete: after it streamed to the end, stopped,
    /// failed or was refused.
    func finish() async {
        live.isStreaming = false
        do {
            try await live.show(response: rawText, streaming: false)
        } catch {
            drawFailure = error.message
        }
    }
}
