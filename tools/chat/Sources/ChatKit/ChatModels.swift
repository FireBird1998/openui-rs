import SwiftUI

/// The openui-cloud template's model menu in upstream order, with #153's
/// additional default in the Anthropic group.
struct ChatModel: Identifiable {
    let id: String
    let name: String

    init(_ id: String, _ name: String) {
        self.id = id
        self.name = name
    }

    struct Group {
        let name: String
        let models: [ChatModel]

        init(_ name: String, _ models: [ChatModel]) {
            self.name = name
            self.models = models
        }
    }

    static let groups = [
        Group("Included", [
            ChatModel("google/gemini-3.1-pro-free", "Gemini 3.1 Pro"),
            ChatModel("google/gemini-3.1-flash-lite-free", "Gemini 3.1 Flash Lite"),
            ChatModel("google/gemini-3.6-flash-free", "Gemini 3.6 Flash"),
            ChatModel("google/gemini-3.5-flash-free", "Gemini 3.5 Flash"),
        ]),
        Group("Anthropic", [
            ChatModel(Template.defaultModel, "Claude Sonnet 5.5"),
            ChatModel("anthropic/claude-sonnet-5", "Claude Sonnet 5"),
            ChatModel("anthropic/claude-sonnet-4.6", "Claude Sonnet 4.6"),
            ChatModel("anthropic/claude-opus-4-7", "Claude Opus 4.7"),
        ]),
        Group("OpenAI", [
            ChatModel("openai/gpt-5.5", "GPT-5.5"),
            ChatModel("openai/gpt-5.4", "GPT-5.4"),
            ChatModel("openai/gpt-5.4-mini", "GPT-5.4 mini"),
            ChatModel("openai/gpt-5.2", "GPT-5.2"),
            ChatModel("openai/gpt-5.1", "GPT-5.1"),
            ChatModel("openai/gpt-5", "GPT-5"),
        ]),
        Group("Google", [
            ChatModel("google/gemini-3.6-flash", "Gemini 3.6 Flash"),
            ChatModel("google/gemini-3.5-flash", "Gemini 3.5 Flash"),
            ChatModel("google/gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview"),
        ]),
    ]

    static func contains(_ id: String) -> Bool {
        groups.contains { group in group.models.contains { $0.id == id } }
    }
}

struct ChatModelSwitcher: View {
    let preferences: ChatPreferences

    var body: some View {
        Picker("Model", selection: Binding(get: { preferences.modelID }, set: { preferences.selectModel($0) })) {
            ForEach(ChatModel.groups, id: \.name) { group in
                Section(group.name) {
                    ForEach(group.models) { model in
                        Text(model.name).tag(model.id)
                    }
                }
            }
        }
        .pickerStyle(.menu)
        .labelsHidden()
    }
}
