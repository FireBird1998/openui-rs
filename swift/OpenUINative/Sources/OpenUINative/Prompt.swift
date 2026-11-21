import Foundation
import OpenUIFFI

/// The options upstream's `generateSystemPrompt` takes beside a library for
/// the full system prompt (its `SystemPromptOptions`). Each one left nil is
/// left out, so upstream's default applies.
public struct PromptOptions: Sendable, Equatable {
    /// Tools the program may call with `Query()` and `Mutation()`.
    public var tools: [PromptTool]?
    /// Ask for edits to an existing program instead of a whole one.
    public var editMode: Bool?
    /// Let the model answer in prose with the program inline.
    public var inlineMode: Bool?
    /// `Query()`, `Mutation()`, `@Run` and the tool workflow; upstream's
    /// default is on when there are tools.
    public var toolCalls: Bool?
    /// `$variables`, `@Set`, `@Reset` and interactive filters; upstream's
    /// default is on with tool calls.
    public var bindings: Bool?
    /// List the built-in functions (`@Count`, `@Sum`, …); upstream's default
    /// is on with tool calls or bindings.
    public var builtinFunctions: Bool?
    /// Text placed before everything else.
    public var preamble: String?
    /// General examples: static and layout patterns.
    public var examples: [String]?
    /// Examples of `Query()` and `Mutation()` patterns.
    public var toolExamples: [String]?
    /// Rules added at the end.
    public var additionalRules: [String]?

    public init(
        tools: [PromptTool]? = nil,
        editMode: Bool? = nil,
        inlineMode: Bool? = nil,
        toolCalls: Bool? = nil,
        bindings: Bool? = nil,
        builtinFunctions: Bool? = nil,
        preamble: String? = nil,
        examples: [String]? = nil,
        toolExamples: [String]? = nil,
        additionalRules: [String]? = nil
    ) {
        self.tools = tools
        self.editMode = editMode
        self.inlineMode = inlineMode
        self.toolCalls = toolCalls
        self.bindings = bindings
        self.builtinFunctions = builtinFunctions
        self.preamble = preamble
        self.examples = examples
        self.toolExamples = toolExamples
        self.additionalRules = additionalRules
    }

    /// The options as upstream's object, in its declared key order.
    var json: JSONValue {
        var entries: [(key: String, value: JSONValue)] = []
        if let tools { entries.append(("tools", .array(tools.map(\.json)))) }
        if let editMode { entries.append(("editMode", .bool(editMode))) }
        if let inlineMode { entries.append(("inlineMode", .bool(inlineMode))) }
        if let toolCalls { entries.append(("toolCalls", .bool(toolCalls))) }
        if let bindings { entries.append(("bindings", .bool(bindings))) }
        if let builtinFunctions { entries.append(("builtinFunctions", .bool(builtinFunctions))) }
        if let preamble { entries.append(("preamble", .string(preamble))) }
        if let examples { entries.append(("examples", .array(examples.map(JSONValue.string)))) }
        if let toolExamples { entries.append(("toolExamples", .array(toolExamples.map(JSONValue.string)))) }
        if let additionalRules { entries.append(("additionalRules", .array(additionalRules.map(JSONValue.string)))) }
        return .object(entries)
    }
}

/// The only options OpenUI Cloud's config block carries (upstream's
/// `CloudPromptOptions`): examples, a preamble and additional rules. Tools,
/// modes and flags have no place in it, so this type has none.
public struct CloudPromptOptions: Sendable, Equatable {
    public var examples: [String]?
    public var preamble: String?
    public var additionalRules: [String]?

    public init(examples: [String]? = nil, preamble: String? = nil, additionalRules: [String]? = nil) {
        self.examples = examples
        self.preamble = preamble
        self.additionalRules = additionalRules
    }

    /// The three options Cloud carries, picked from the full prompt's, as
    /// upstream picks them; the rest are dropped.
    public init(_ options: PromptOptions) {
        self.init(examples: options.examples, preamble: options.preamble, additionalRules: options.additionalRules)
    }

    var json: JSONValue {
        var entries: [(key: String, value: JSONValue)] = []
        if let examples { entries.append(("examples", .array(examples.map(JSONValue.string)))) }
        if let preamble { entries.append(("preamble", .string(preamble))) }
        if let additionalRules { entries.append(("additionalRules", .array(additionalRules.map(JSONValue.string)))) }
        return .object(entries)
    }
}

/// A tool named in a prompt: by name alone, or described (upstream's
/// `string | ToolSpec`).
public enum PromptTool: Sendable, Equatable {
    case name(String)
    case spec(ToolSpec)

    var json: JSONValue {
        switch self {
        case let .name(name): .string(name)
        case let .spec(spec): spec.json
        }
    }
}

/// A tool the model can use through `Query()` and `Mutation()`, shaped like
/// an MCP tool (upstream's `ToolSpec`).
public struct ToolSpec: Sendable, Equatable {
    public var name: String
    public var description: String?
    /// The arguments' JSON Schema.
    public var inputSchema: JSONValue
    /// The result's JSON Schema; nil leaves it out.
    public var outputSchema: JSONValue?
    public var annotations: ToolAnnotations?

    public init(name: String, description: String? = nil, inputSchema: JSONValue,
                outputSchema: JSONValue? = nil, annotations: ToolAnnotations? = nil) {
        self.name = name
        self.description = description
        self.inputSchema = inputSchema
        self.outputSchema = outputSchema
        self.annotations = annotations
    }

    var json: JSONValue {
        var entries: [(key: String, value: JSONValue)] = [("name", .string(name))]
        if let description { entries.append(("description", .string(description))) }
        entries.append(("inputSchema", inputSchema))
        if let outputSchema { entries.append(("outputSchema", outputSchema)) }
        if let annotations { entries.append(("annotations", annotations.json)) }
        return .object(entries)
    }
}

/// What a tool says about itself, as MCP's tool annotations do.
public struct ToolAnnotations: Sendable, Equatable {
    public var readOnlyHint: Bool?
    public var destructiveHint: Bool?

    public init(readOnlyHint: Bool? = nil, destructiveHint: Bool? = nil) {
        self.readOnlyHint = readOnlyHint
        self.destructiveHint = destructiveHint
    }

    var json: JSONValue {
        var entries: [(key: String, value: JSONValue)] = []
        if let readOnlyHint { entries.append(("readOnlyHint", .bool(readOnlyHint))) }
        if let destructiveHint { entries.append(("destructiveHint", .bool(destructiveHint))) }
        return .object(entries)
    }
}

extension Core {
    /// The full system prompt for a component library, as upstream's
    /// `generateSystemPrompt({library, promptOptions})` gives it. `library`
    /// is the library's prompt spec (upstream's `library.toSpec()`).
    public func systemPrompt(library: JSONValue, options: PromptOptions = PromptOptions()) async throws(OpenUIError) -> String {
        try await systemPrompt(spec: JSONValue.object([("library", library), ("promptOptions", options.json)]).compact)
    }

    /// OpenUI Cloud's config block, which takes the place of the system
    /// prompt in cloud prompt mode, as upstream's
    /// `generateSystemPrompt({cloud: true, library, promptOptions, instructions})`
    /// gives it: the library without its components, and only the options
    /// Cloud carries. With no `library`, Cloud's built-in chat library is
    /// named and options are refused (`.prompt`), as upstream refuses them.
    /// `instructions` is prose appended after the block.
    public func cloudConfigBlock(
        library: JSONValue?,
        options: CloudPromptOptions = CloudPromptOptions(),
        instructions: String? = nil
    ) async throws(OpenUIError) -> String {
        var spec: [(key: String, value: JSONValue)] = [("cloud", .bool(true))]
        if let library { spec.append(("library", library)) }
        spec.append(("promptOptions", options.json))
        if let instructions { spec.append(("instructions", .string(instructions))) }
        return try await systemPrompt(spec: JSONValue.object(spec).compact)
    }
}
