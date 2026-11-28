package dev.openui.compose

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * The recorded upstream fixtures (conformance/golden, written by
 * scripts/regen-golden.sh running upstream's own code), and the typed
 * Kotlin declarations that say exactly what a recorded plain-data
 * definition says. The tests run in kotlin/OpenUICompose, two folders down.
 */
internal object Recorded {
    private val repository: File = checkNotNull(File(checkNotNull(System.getProperty("user.dir"))).absoluteFile.parentFile?.parentFile) {
        "the tests run two folders down from the repository"
    }

    fun golden(name: String): JsonElement {
        val file = File(repository, "conformance/golden/$name")
        check(file.isFile) { "no recorded fixture at $file" }
        return Json.parseToJsonElement(file.readText())
    }

    /**
     * [json] as JavaScript's `JSON.stringify` writes it, which is how
     * upstream's text was made: keys in the order the recording keeps
     * (JavaScript's own), no spaces, and JavaScript's string escapes.
     */
    fun stringify(json: JsonElement): String = StringBuilder().also { write(json, it) }.toString()

    private fun write(json: JsonElement, out: StringBuilder) {
        when (json) {
            is JsonObject -> {
                out.append('{')
                json.entries.forEachIndexed { i, (key, value) ->
                    if (i > 0) out.append(',')
                    quote(key, out)
                    out.append(':')
                    write(value, out)
                }
                out.append('}')
            }
            is JsonArray -> {
                out.append('[')
                json.forEachIndexed { i, value ->
                    if (i > 0) out.append(',')
                    write(value, out)
                }
                out.append(']')
            }
            is JsonPrimitive -> if (json.isString) quote(json.content, out) else out.append(json.content)
        }
    }

    private fun quote(text: String, out: StringBuilder) {
        out.append('"')
        for ((i, c) in text.withIndex()) {
            when {
                c == '"' -> out.append("\\\"")
                c == '\\' -> out.append("\\\\")
                c == '\b' -> out.append("\\b")
                c == '\u000C' -> out.append("\\f")
                c == '\n' -> out.append("\\n")
                c == '\r' -> out.append("\\r")
                c == '\t' -> out.append("\\t")
                c < ' ' -> out.append("\\u%04x".format(c.code))
                c.isHighSurrogate() && !(i + 1 < text.length && text[i + 1].isLowSurrogate()) ->
                    out.append("\\u%04x".format(c.code))
                c.isLowSurrogate() && !(i > 0 && text[i - 1].isHighSurrogate()) ->
                    out.append("\\u%04x".format(c.code))
                else -> out.append(c)
            }
        }
        out.append('"')
    }
}

/** A recorded definition holds something the typed declarations can't say. */
internal class Untyped(what: String) : Exception(what)

/** A recorded definition, said with the typed declarations. */
internal class TypedDefinition(
    val components: List<DefinedComponent>,
    val groups: List<ComponentGroup>?,
    val root: String?,
) {
    fun create(root: String? = this.root): Library = createLibrary(components, groups, root)
}

private fun JsonObject.onlyKeys(vararg allowed: String) {
    keys.firstOrNull { it !in allowed }?.let { throw Untyped("key $it") }
}

private fun JsonElement.string(): String {
    val primitive = this as? JsonPrimitive ?: throw Untyped("not a string: $this")
    if (!primitive.isString) throw Untyped("not a string: $this")
    return primitive.content
}

private fun JsonElement.strings(): List<String> = (this as? JsonArray ?: throw Untyped("not a list: $this")).map { it.string() }

/** `true` or absent; the typed declarations write nothing for false. */
private fun JsonObject.flag(key: String): Boolean {
    val value = this[key] ?: return false
    if ((value as? JsonPrimitive)?.takeUnless { it.isString }?.booleanOrNull != true) throw Untyped("$key: $value")
    return true
}

internal fun typedType(json: JsonElement): PropType {
    val type = json as? JsonObject ?: throw Untyped("type $json")
    return when (val kind = type["kind"]?.string()) {
        "string" -> PropType.string.also { type.onlyKeys("kind") }
        "number" -> PropType.number.also { type.onlyKeys("kind") }
        "boolean" -> PropType.boolean.also { type.onlyKeys("kind") }
        "any" -> PropType.any.also { type.onlyKeys("kind") }
        "enum" -> { type.onlyKeys("kind", "values"); PropType.words(type.getValue("values").strings()) }
        "array" -> { type.onlyKeys("kind", "items"); PropType.array(typedType(type.getValue("items"))) }
        "union" -> {
            type.onlyKeys("kind", "options")
            PropType.union(type.getValue("options").jsonArray.map(::typedType))
        }
        "object" -> {
            type.onlyKeys("kind", "fields")
            PropType.objectOf(type.getValue("fields").jsonArray.map { field ->
                val f = field.jsonObject
                f.onlyKeys("name", "type", "optional")
                FieldDefinition(f.getValue("name").string(), typedType(f.getValue("type")), optional = f.flag("optional"))
            })
        }
        "record" -> { type.onlyKeys("kind", "values"); PropType.record(typedType(type.getValue("values"))) }
        "nullable" -> { type.onlyKeys("kind", "type"); PropType.nullable(typedType(type.getValue("type"))) }
        "component" -> { type.onlyKeys("kind", "name"); PropType.component(type.getValue("name").string()) }
        "action" -> { type.onlyKeys("kind", "name"); PropType.action(type["name"]?.string()) }
        "named" -> {
            type.onlyKeys("kind", "name", "type")
            PropType.named(type.getValue("name").string(), typedType(type.getValue("type")))
        }
        else -> throw Untyped("kind $kind")
    }
}

/** The typed declarations for a recorded definition, or [Untyped] if they can't say it exactly. */
internal fun typedDefinition(json: JsonElement): TypedDefinition {
    val definition = json.jsonObject
    definition.onlyKeys("components", "componentGroups", "root")
    val components = definition.getValue("components").jsonArray.map { component ->
        val c = component.jsonObject
        c.onlyKeys("name", "description", "props")
        defineComponent(c.getValue("name").string(), c.getValue("description").string(),
            c.getValue("props").jsonArray.map { prop ->
                val p = prop.jsonObject
                p.onlyKeys("name", "type", "optional", "binding")
                PropDefinition(p.getValue("name").string(), typedType(p.getValue("type")),
                    optional = p.flag("optional"), binding = p.flag("binding"))
            })
    }
    val groups = definition["componentGroups"]?.let { groups ->
        if (groups == JsonNull) throw Untyped("null groups")
        groups.jsonArray.map { group ->
            val g = group.jsonObject
            g.onlyKeys("name", "components", "notes")
            ComponentGroup(g.getValue("name").string(), g.getValue("components").strings(), g["notes"]?.strings())
        }
    }
    return TypedDefinition(components, groups, definition["root"]?.string())
}

private fun JsonObject.bool(key: String): Boolean? = this[key]?.let {
    (it as? JsonPrimitive)?.takeUnless { p -> p.isString }?.booleanOrNull ?: throw Untyped("$key: $it")
}

private fun typedTool(json: JsonElement): PromptTool {
    if (json is JsonPrimitive) return PromptTool.Name(json.string())
    val tool = json as? JsonObject ?: throw Untyped("tool $json")
    tool.onlyKeys("name", "description", "inputSchema", "outputSchema", "annotations")
    val annotations = tool["annotations"]?.let { a ->
        val hints = a as? JsonObject ?: throw Untyped("annotations $a")
        hints.onlyKeys("readOnlyHint", "destructiveHint")
        ToolAnnotations(hints.bool("readOnlyHint"), hints.bool("destructiveHint"))
    }
    return PromptTool.Spec(ToolSpec(
        name = tool.getValue("name").string(),
        description = tool["description"]?.string(),
        inputSchema = tool["inputSchema"] ?: throw Untyped("no inputSchema"),
        outputSchema = tool["outputSchema"],
        annotations = annotations,
    ))
}

/** The typed options that say exactly what a recorded `options` or `promptOptions` says. */
internal fun typedOptions(json: JsonElement?): PromptOptions {
    val options = (json ?: return PromptOptions()) as? JsonObject ?: throw Untyped("options $json")
    options.onlyKeys("tools", "editMode", "inlineMode", "toolCalls", "bindings", "builtinFunctions", "preamble",
        "examples", "toolExamples", "additionalRules")
    return PromptOptions(
        tools = options["tools"]?.let { (it as? JsonArray ?: throw Untyped("tools $it")).map(::typedTool) },
        editMode = options.bool("editMode"),
        inlineMode = options.bool("inlineMode"),
        toolCalls = options.bool("toolCalls"),
        bindings = options.bool("bindings"),
        builtinFunctions = options.bool("builtinFunctions"),
        preamble = options["preamble"]?.string(),
        examples = options["examples"]?.strings(),
        toolExamples = options["toolExamples"]?.strings(),
        additionalRules = options["additionalRules"]?.strings(),
    )
}

internal val JsonElement.text: String? get() = (this as? JsonPrimitive)?.takeIf { it.isString }?.content

internal fun JsonElement.field(key: String): JsonElement? = (this as? JsonObject)?.get(key)

internal fun JsonElement.name(): String = jsonObject.getValue("name").jsonPrimitive.content
