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

/// What the maintainer asks for: a request, a model, an effort and a library.
struct LiveDraft {
    var text: String
    var model: ClaudeModel
    var effort: String
    var library: String
}

/// Asks Claude for a screen: the request, the model, effort and component
/// library, with the budget and whether a key is saved.
struct AskClaudeSheet: View {
    /// Libraries the corpus has a prompt spec for.
    let libraries: [String]
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
            Text("Ask Claude for a screen").font(.title2.bold())
            Text("Claude writes OpenUI Lang for the component library, and the viewer draws it as it streams.")
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
                Picker("Component library", selection: $library) {
                    ForEach(libraries, id: \.self) { Text($0).tag($0) }
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
                    ask(LiveDraft(text: text, model: model, effort: effort, library: library))
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
            if library.isEmpty { library = libraries.first ?? "" }
        }
    }

    static func price(_ dollars: Double) -> String {
        dollars < 1 ? String(format: "%.2f", dollars) : String(format: "%.0f", dollars)
    }
}

/// A live request's progress, cost and a stop button.
struct LiveBar: View {
    let status: LiveStatus
    let stop: () -> Void

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
