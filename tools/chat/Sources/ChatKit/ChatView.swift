import OpenUINative
import SwiftUI

/// The app's window: load the library and open a workspace for the saved key.
public struct ChatWindow: View {
    @State private var library: ChatLibrary?
    @State private var workspace: ChatThreads?
    @State private var loadFailure: String?
    private let keySettings: KeySettings

    public init(keySettings: KeySettings = KeySettings(store: KeychainKeyStore())) {
        self.keySettings = keySettings
    }

    public var body: some View {
        Group {
            if let workspace {
                ThreadWorkspace(chats: workspace)
            } else if let loadFailure {
                ContentUnavailableView("The library did not load", systemImage: "exclamationmark.triangle",
                                       description: Text(loadFailure))
            } else {
                ProgressView()
            }
        }
        .frame(minWidth: 720, minHeight: 480)
        .task(id: keySettings.revision) {
            workspace?.thread.stop()
            workspace = nil
            loadFailure = nil
            do {
                if library == nil { library = try await ChatLibrary.reactUI() }
                try Task.checkCancellation()
                guard let library else { return }
                let chats = ChatThreads(library: library, gateway: ThesysGateway(), storage: ThesysConversations())
                workspace = chats
                if keySettings.hasKey { await chats.loadThreads() }
            } catch is CancellationError {
            } catch {
                loadFailure = ChatThread.message(for: error)
            }
        }
    }
}

/// One thread: the welcome screen or the turns, the error callout, and the
/// composer.
public struct ChatView: View {
    @Bindable var thread: ChatThread

    public init(thread: ChatThread) {
        self.thread = thread
    }

    public var body: some View {
        VStack(spacing: 0) {
            if thread.turns.isEmpty && thread.error == nil {
                Welcome(thread: thread)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        ForEach(thread.turns) { turn in
                            switch turn {
                            case let .user(message): UserBubble(turn: message)
                            case let .assistant(answer): AnswerBubble(answer: answer)
                            }
                        }
                        if let error = thread.error {
                            ErrorCallout(message: error)
                        }
                    }
                    .padding(20)
                }
                .defaultScrollAnchor(.bottom)
            }
            Composer(thread: thread)
        }
    }
}

private struct Welcome: View {
    let thread: ChatThread

    var body: some View {
        VStack(spacing: 20) {
            Spacer()
            Text(Template.welcomeTitle).font(.largeTitle.weight(.semibold))
            HStack(spacing: 12) {
                ForEach(Template.starters, id: \.displayText) { starter in
                    Button {
                        thread.send(starter.prompt)
                    } label: {
                        Label(starter.displayText, systemImage: "lightbulb")
                            .frame(maxWidth: 180, minHeight: 44)
                    }
                    .buttonStyle(.bordered)
                }
            }
            Spacer()
        }
        .padding(20)
        .frame(maxWidth: .infinity)
    }
}

struct UserBubble: View {
    let turn: UserTurn

    var body: some View {
        HStack {
            Spacer(minLength: 80)
            VStack(alignment: .leading, spacing: 6) {
                if let form = turn.formData {
                    DisclosureGroup(form.title) {
                        VStack(alignment: .leading, spacing: 4) {
                            ForEach(Array(form.fields.enumerated()), id: \.offset) { _, field in
                                HStack(alignment: .firstTextBaseline, spacing: 4) {
                                    Text("\(field.label):").foregroundStyle(.secondary)
                                    Text(field.value).textSelection(.enabled)
                                }
                            }
                        }
                        .padding(.top, 4)
                    }
                }
                if !turn.text.isEmpty {
                    Text(turn.text).textSelection(.enabled)
                }
            }
            .padding(.horizontal, 14).padding(.vertical, 10)
            .background(.tint.opacity(0.15), in: .rect(cornerRadius: 14))
        }
    }
}

/// An answer drawn from its program; text outside the program is not
/// shown, and an answer with no program is an empty bubble, as upstream. An
/// answer that could not be drawn shows why under it.
private struct AnswerBubble: View {
    let answer: AssistantTurn

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ElementView(live: answer.live)
            // The template has no failed drawing; the app shows why in the
            // same danger callout it uses for a failed request.
            if let failure = answer.drawFailure {
                ErrorCallout(message: failure)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 24, alignment: .leading)
    }
}

/// react-ui's danger callout with the thread's error.
struct ErrorCallout: View {
    let message: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.octagon.fill").foregroundStyle(.red)
            VStack(alignment: .leading, spacing: 4) {
                Text(Template.errorTitle).font(.headline)
                Text(message).textSelection(.enabled)
            }
            Spacer(minLength: 0)
        }
        .padding(12)
        .background(.red.opacity(0.1), in: .rect(cornerRadius: 10))
        .accessibilityElement(children: .combine)
    }
}

/// Upstream's composer: Return sends, Shift-Return starts a new line, and
/// the send button is a stop button while an answer runs.
private struct Composer: View {
    @Bindable var thread: ChatThread

    var body: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField(Template.placeholder, text: $thread.draft, axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1...8)
                .onKeyPress(.return, phases: .down) { press in
                    if press.modifiers.contains(.shift) {
                        thread.draft += "\n"
                    } else {
                        thread.sendDraft()
                    }
                    return .handled
                }
            Button {
                if thread.isRunning { thread.stop() } else { thread.sendDraft() }
            } label: {
                Image(systemName: thread.isRunning ? "stop.fill" : "arrow.up")
                    .frame(width: 18, height: 18)
            }
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.circle)
            .accessibilityLabel(thread.isRunning ? "Cancel message" : "Send message")
        }
        .padding(12)
        .background(.background.secondary, in: .rect(cornerRadius: 16))
        .padding(12)
    }
}
