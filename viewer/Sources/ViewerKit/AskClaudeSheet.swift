import SwiftUI

/// Spending on live requests, kept in the app's settings: the cap the
/// maintainer sets, and the estimated total so far.
public enum Budget {
    public static let capKey = "budgetCapDollars"
    public static let spentKey = "spentDollars"
    public static let defaultCap = 10.0

    public static var cap: Double {
        UserDefaults.standard.object(forKey: capKey) as? Double ?? defaultCap
    }

    public static var spent: Double {
        UserDefaults.standard.double(forKey: spentKey)
    }

    public static var reached: Bool { spent >= cap }

    static func add(_ dollars: Double) {
        UserDefaults.standard.set(spent + dollars, forKey: spentKey)
    }

    static func format(_ dollars: Double) -> String {
        String(format: dollars < 0.1 ? "$%.4f" : "$%.2f", dollars)
    }
}

/// What the maintainer asks for: a request, a model, an effort and a library,
/// and for a follow-up, the program to change.
struct LiveDraft {
    var text: String
    var model: ClaudeModel
    var effort: String
    var library: String
    var base: String?
}

/// Asks Claude for a screen: the request, the model, effort and component
/// library, with the budget and whether a key is saved. For a follow-up, it
/// asks for a change to the program on screen, in its library.
struct AskClaudeSheet: View {
    /// Libraries the corpus has a prompt spec for.
    let libraries: [String]
    /// The program to change, for a follow-up.
    var base: String?
    /// The follow-up's library: the program's own.
    var baseLibrary: String?
    let ask: (LiveDraft) -> Void
    @Environment(\.dismiss) private var dismiss
    @AppStorage("liveModel") private var modelID = ClaudeModel.opus.id
    @AppStorage("liveEffort") private var effort = "medium"
    @AppStorage(Budget.capKey) private var cap = Budget.defaultCap
    @AppStorage(Budget.spentKey) private var spent = 0.0
    @State private var text = ""
    @State private var library = ""
    @State private var hasKey = APIKeyStore.hasKey

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(base == nil ? "Ask Claude for a screen" : "Ask Claude to change this screen").font(.title2.bold())
            Text(base == nil
                 ? "Claude writes OpenUI Lang for the component library, and the viewer draws it as it streams."
                 : "Claude answers in edit mode with a patch, only the statements that change, and the viewer merges it into the screen as it streams.")
                .foregroundStyle(.secondary)
            TextEditor(text: $text)
                .font(.body)
                .frame(minHeight: 120)
                .border(.separator)
            Form {
                Picker("Model", selection: $modelID) {
                    ForEach(ClaudeModel.all) { model in
                        Text("\(model.name) ($\(Self.price(model.inputPrice)) / $\(Self.price(model.outputPrice)) per million tokens)").tag(model.id)
                    }
                }
                Picker("Effort", selection: $effort) {
                    Text("Low").tag("low")
                    Text("Medium").tag("medium")
                    Text("High").tag("high")
                }
                .pickerStyle(.segmented)
                if base == nil {
                    Picker("Component library", selection: $library) {
                        ForEach(libraries, id: \.self) { Text($0).tag($0) }
                    }
                } else {
                    LabeledContent("Component library", value: library)
                }
            }
            VStack(alignment: .leading, spacing: 4) {
                Text("Spent so far: \(Budget.format(spent)) of a \(Budget.format(cap)) cap (estimated). Each request is capped at 16,000 output tokens.")
                    .font(.callout).foregroundStyle(spent >= cap ? .red : .secondary)
                if !hasKey {
                    Text("No API key saved. Add one in Settings (⌘,); it's kept in your Keychain.")
                        .font(.callout).foregroundStyle(.orange)
                }
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Ask") {
                    let model = ClaudeModel.all.first { $0.id == modelID } ?? .opus
                    ask(LiveDraft(text: text, model: model, effort: effort, library: library, base: base))
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !hasKey || spent >= cap || library.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 620)
        .onAppear {
            hasKey = APIKeyStore.hasKey
            if let baseLibrary { library = baseLibrary } else if library.isEmpty { library = libraries.first ?? "" }
        }
    }

    static func price(_ dollars: Double) -> String {
        dollars < 1 ? String(format: "%.2f", dollars) : String(format: "%.0f", dollars)
    }
}

/// A live request's progress, cost, a stop button, and once it's over, a
/// button to save it as a recording.
struct LiveBar: View {
    let status: LiveStatus
    let stop: () -> Void
    let save: () -> Void
    /// The recording's file, once saved.
    let saved: URL?
    /// Asks for a change to the screen, once the request is over; nil when it can't.
    let followUp: (() -> Void)?

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: icon).foregroundStyle(color)
            Text(message).foregroundStyle(color)
            if let fallback = status.fallback {
                Text("(\(fallback))").foregroundStyle(.secondary)
            }
            Spacer()
            Text("\(status.inputTokens) in / \(status.outputTokens > 0 ? "\(status.outputTokens)" : "…") out · \(Budget.format(status.cost))")
                .monospacedDigit()
                .foregroundStyle(.secondary)
                .help("Estimated cost. Output tokens arrive at the end; until then the estimate counts text only.")
            if status.isActive {
                Button("Stop", systemImage: "stop.fill", action: stop)
            } else if let saved {
                Button("Saved", systemImage: "checkmark") { Recordings.reveal(saved) }
                    .help("Show the recording in Finder")
            } else if !status.received.isEmpty {
                Button("Save Recording", systemImage: "record.circle", action: save)
            }
            if !status.isActive, let followUp {
                Button("Follow Up", systemImage: "text.bubble", action: followUp)
                    .help("Ask Claude to change this screen; it answers with a patch")
            }
        }
        .font(.callout)
    }

    private var message: String {
        let model = status.servedBy ?? status.model.id
        switch status.phase {
        case .connecting: return "Asking \(model)…"
        case .thinking: return "\(model) is thinking…"
        case .writing: return "\(model) is writing…"
        case let .finished(reason):
            switch reason {
            case "end_turn": return "Done (\(model))"
            case "max_tokens": return "Stopped at the 16,000-token limit"
            case "refusal": return "\(model) declined this request"
            default: return "Finished: \(reason)"
            }
        case let .failed(message): return message
        case .cancelled: return "Stopped"
        }
    }

    private var icon: String {
        switch status.phase {
        case .connecting, .thinking, .writing: "dot.radiowaves.left.and.right"
        case .finished("end_turn"): "checkmark.circle"
        case .finished: "exclamationmark.triangle"
        case .failed: "xmark.octagon"
        case .cancelled: "stop.circle"
        }
    }

    private var color: Color {
        switch status.phase {
        case .finished("end_turn"), .connecting, .thinking, .writing: .primary
        case .failed: .red
        default: .orange
        }
    }
}

/// A recording's details, with buttons to replay it with its original timing
/// and to show its file.
struct RecordingBar: View {
    let file: URL?
    let recording: Recording
    let replay: () -> Void
    let followUp: (() -> Void)?

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "record.circle").foregroundStyle(.secondary)
            Text(summary).foregroundStyle(.secondary).lineLimit(1)
            Spacer()
            Button("Replay as Recorded", systemImage: "clock.arrow.circlepath", action: replay)
                .help("Play the chunks again with the timing they arrived with")
            if let file {
                Button("Show in Finder", systemImage: "folder") { Recordings.reveal(file) }
            }
            if let followUp {
                Button("Follow Up", systemImage: "text.bubble", action: followUp)
            }
        }
        .font(.callout)
    }

    private var summary: String {
        let seconds = (recording.chunks.last?.t ?? 0) - (recording.chunks.first?.t ?? 0)
        let date = recording.createdAt.formatted(date: .abbreviated, time: .shortened)
        let ending = recording.stopReason.map { " · \($0)" } ?? " · stopped early"
        let kind = recording.patchOf == nil ? "" : "follow-up · "
        return "\(kind)\(recording.servedBy ?? recording.model) · \(date) · \(recording.chunks.count) chunks over \(String(format: "%.1f", seconds)) s\(ending)"
    }
}
