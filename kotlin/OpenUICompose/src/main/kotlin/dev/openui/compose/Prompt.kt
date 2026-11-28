package dev.openui.compose

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * The options upstream's `library.prompt` takes for the full system prompt
 * (its `PromptOptions`). Each one left null is left out, so upstream's
 * default applies.
 */
public data class PromptOptions(
    /** Tools the program may call with `Query()` and `Mutation()`. */
    public val tools: List<PromptTool>? = null,
    /** Ask for edits to an existing program instead of a whole one. */
    public val editMode: Boolean? = null,
    /** Let the model answer in prose with the program inline. */
    public val inlineMode: Boolean? = null,
    /** `Query()`, `Mutation()`, `@Run` and the tool workflow; upstream's default is on when there are tools. */
    public val toolCalls: Boolean? = null,
    /** `$variables`, `@Set`, `@Reset` and interactive filters; upstream's default is on with tool calls. */
    public val bindings: Boolean? = null,
    /** List the built-in functions (`@Count`, `@Sum`, …); upstream's default is on with tool calls or bindings. */
    public val builtinFunctions: Boolean? = null,
    /** Text placed before everything else. */
    public val preamble: String? = null,
    /** General examples: static and layout patterns. */
    public val examples: List<String>? = null,
    /** Examples of `Query()` and `Mutation()` patterns. */
    public val toolExamples: List<String>? = null,
    /** Rules added at the end. */
    public val additionalRules: List<String>? = null,
) {
    /** The options as upstream's object, in its declared key order. */
    internal val json: JsonObject
        get() = buildJsonObject {
            tools?.let { put("tools", JsonArray(it.map { tool -> tool.json })) }
            editMode?.let { put("editMode", it) }
            inlineMode?.let { put("inlineMode", it) }
            toolCalls?.let { put("toolCalls", it) }
            bindings?.let { put("bindings", it) }
            builtinFunctions?.let { put("builtinFunctions", it) }
            preamble?.let { put("preamble", it) }
            putStrings("examples", examples)
            putStrings("toolExamples", toolExamples)
            putStrings("additionalRules", additionalRules)
        }
}

/**
 * The only options OpenUI Cloud's config block carries (upstream's
 * `CloudPromptOptions`): examples, a preamble and additional rules. Tools,
 * modes and flags have no place in it, so this type has none.
 */
public data class CloudPromptOptions(
    public val examples: List<String>? = null,
    public val preamble: String? = null,
    public val additionalRules: List<String>? = null,
) {
    /** The three options Cloud carries, picked from the full prompt's, as upstream picks them; the rest are dropped. */
    public constructor(options: PromptOptions) :
        this(examples = options.examples, preamble = options.preamble, additionalRules = options.additionalRules)

    internal val json: JsonObject
        get() = buildJsonObject {
            putStrings("examples", examples)
            preamble?.let { put("preamble", it) }
            putStrings("additionalRules", additionalRules)
        }
}

/** A tool named in a prompt: by name alone, or described (upstream's `string | ToolSpec`). */
public sealed interface PromptTool {
    public data class Name(public val name: String) : PromptTool

    public data class Spec(public val spec: ToolSpec) : PromptTool
}

private val PromptTool.json: JsonElement
    get() = when (this) {
        is PromptTool.Name -> JsonPrimitive(name)
        is PromptTool.Spec -> spec.json
    }

/**
 * A tool the model can use through `Query()` and `Mutation()`, shaped like
 * an MCP tool (upstream's `ToolSpec`). The schemas cross to the core as JSON
 * text: a non-finite number in them (NaN, an infinity) is written as `null`,
 * as `JSON.stringify` writes it, and a non-string value whose text isn't a
 * JSON literal (a `Number` subclass's own `toString()`, say) is refused with
 * [OpenUIException.Kind.Decode].
 */
public data class ToolSpec(
    public val name: String,
    public val description: String? = null,
    /** The arguments' JSON Schema. */
    public val inputSchema: JsonElement,
    /** The result's JSON Schema; null leaves it out. */
    public val outputSchema: JsonElement? = null,
    public val annotations: ToolAnnotations? = null,
) {
    internal val json: JsonObject
        get() = buildJsonObject {
            put("name", name)
            description?.let { put("description", it) }
            put("inputSchema", inputSchema)
            outputSchema?.let { put("outputSchema", it) }
            annotations?.let { put("annotations", it.json) }
        }
}

/** What a tool says about itself, as MCP's tool annotations do. */
public data class ToolAnnotations(
    public val readOnlyHint: Boolean? = null,
    public val destructiveHint: Boolean? = null,
) {
    internal val json: JsonObject
        get() = buildJsonObject {
            readOnlyHint?.let { put("readOnlyHint", it) }
            destructiveHint?.let { put("destructiveHint", it) }
        }
}

private fun JsonObjectBuilder.putStrings(key: String, values: List<String>?) {
    values?.let { put(key, JsonArray(it.map(::JsonPrimitive))) }
}

/**
 * The full system prompt for `library`, byte for byte as upstream's
 * `library.prompt(options)` gives it. The core generates it; a prompt it
 * can't make is refused with [OpenUIException.Kind.Prompt], with upstream's
 * message. The options cross to the core as JSON text, under the core's host
 * input limit: past it they are refused with the core's
 * [OpenUIException.Kind.LimitExceeded]. Options the core can't read are
 * refused with [OpenUIException.Kind.InvalidPromptSpec]: a tool schema nested
 * past the core's JSON depth limit, say, or a number out of its range such as
 * `1E400`. A tool's schemas cross as [ToolSpec] says.
 */
public fun systemPrompt(library: Library, options: PromptOptions = PromptOptions()): String =
    checkNotNull(core { library.inner.prompt(jsonText(options.json)) }) { "a library made from a definition has a prompt" }

/**
 * OpenUI Cloud's config block for `library`, which takes the place of the
 * system prompt in cloud prompt mode, byte for byte as upstream's
 * `generateSystemPrompt({cloud: true, library: library.toSpec(),
 * promptOptions, instructions})` gives it: the library without its
 * components, and only the options Cloud carries. `instructions` is prose
 * placed after the block. A library Cloud doesn't accept is refused with
 * [OpenUIException.Kind.Prompt], with upstream's message.
 */
public fun cloudConfigBlock(
    library: Library,
    options: CloudPromptOptions = CloudPromptOptions(),
    instructions: String? = null,
): String =
    checkNotNull(core { library.inner.cloudConfig(jsonText(options.json), instructions) }) {
        "a library made from a definition has a cloud config block"
    }
