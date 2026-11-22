import Foundation
import OpenUINative

/// The component library answers are drawn with, and the system prompt that
/// asks for it. The first version uses react-ui's library and its prompt
/// options in cloud prompt mode, as upstream's openui-cloud template does
/// (#129, amendment 1). The two files in `Library/` are copies of the
/// corpus's `conformance/imported/schemas/react-ui.json` (the library's JSON
/// Schema) and `conformance/prompts/react-ui-schema.json` (its prompt spec
/// and prompt options, the same as the template's `src/generated/spec.json`);
/// a test keeps them equal.
public struct ChatLibrary: Sendable {
    public let core: Core
    public let library: Library
    /// The prompt spec, which says which props are bindings.
    public let promptSpec: String
    /// The cloud config block that is sent as the system message.
    public let systemPrompt: String

    /// react-ui's library, with the system prompt the template builds with
    /// `generateSystemPrompt({cloud: true, library, promptOptions})`.
    public static func reactUI(core: Core = Core()) async throws -> ChatLibrary {
        let schema = try resource("react-ui-schema")
        let promptSpec = try resource("react-ui-spec")
        let spec: JSONValue
        do { spec = try JSONValue.parse(promptSpec) } catch { throw OpenUIError(.invalidPromptSpec, error.description) }
        let options = spec["promptOptions"]
        let strings: (String) -> [String]? = { key in options?[key]?.array?.compactMap(\.string) }
        let systemPrompt = try await core.cloudConfigBlock(
            library: spec["library"],
            options: CloudPromptOptions(examples: strings("examples"), preamble: options?["preamble"]?.string,
                                        additionalRules: strings("additionalRules")))
        return ChatLibrary(core: core, library: try await core.library(schema: schema),
                           promptSpec: promptSpec, systemPrompt: systemPrompt)
    }

    static func resource(_ name: String) throws -> String {
        guard let url = Bundle.module.url(forResource: name, withExtension: "json", subdirectory: "Library") else {
            throw OpenUIError(.invalidSchema, "the app has no \(name).json")
        }
        return try String(contentsOf: url, encoding: .utf8)
    }
}
