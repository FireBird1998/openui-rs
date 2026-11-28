package dev.openui.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import dev.openui.compose.ffi.ComponentLibrary
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * A prop's type in a library definition, in the plain-data form the core
 * reads (ADR 0017, `docs/library-definition.md`). The core turns it into
 * the library's JSON Schema and prompt signature, as upstream's
 * `createLibrary` does for the zod type; nothing here checks a type itself.
 * Whether a prop is optional or a binding belongs to the prop, not its type.
 */
public class PropType private constructor(internal val json: JsonObject) {
    private constructor(kind: String, fields: Map<String, JsonElement> = emptyMap()) :
        this(JsonObject(mapOf("kind" to JsonPrimitive(kind)) + fields))

    public companion object {
        /** Text: upstream's `z.string()`. */
        public val string: PropType = PropType("string")

        /** A number: `z.number()`. */
        public val number: PropType = PropType("number")

        /** `true` or `false`: `z.boolean()`. */
        public val boolean: PropType = PropType("boolean")

        /** Any value: `z.any()`. */
        public val any: PropType = PropType("any")

        /** One of these words: `z.enum(values)`. */
        public fun words(values: List<String>): PropType =
            PropType("enum", mapOf("values" to JsonArray(values.map(::JsonPrimitive))))

        /** A list of `items`: `z.array(items)`. */
        public fun array(items: PropType): PropType = PropType("array", mapOf("items" to items.json))

        /** Any one of `options`, tried in order: `z.union(options)`. */
        public fun union(options: List<PropType>): PropType =
            PropType("union", mapOf("options" to JsonArray(options.map { it.json })))

        /**
         * An object with these fields, in this order: `z.object({...})`.
         * OpenUINative calls it `object`, a word Kotlin keeps for itself.
         */
        public fun objectOf(fields: List<FieldDefinition>): PropType =
            PropType("object", mapOf("fields" to JsonArray(fields.map { it.json })))

        /** An object with any keys, each holding `values`: `z.record(z.string(), values)`. */
        public fun record(values: PropType): PropType = PropType("record", mapOf("values" to values.json))

        /** `type` or `null`: `type.nullable()`. */
        public fun nullable(type: PropType): PropType = PropType("nullable", mapOf("type" to type.json))

        /** An element of the component called `name`, declared in the library or not. */
        public fun component(name: String): PropType = PropType("component", mapOf("name" to JsonPrimitive(name)))

        /**
         * An action: any action plan, or with `name` the one action of that
         * name (a built-in such as `OpenUrl`, or one the library declares).
         */
        public fun action(name: String? = null): PropType =
            PropType("action", name?.let { mapOf("name" to JsonPrimitive(it)) } ?: emptyMap())

        /** `type` under a name the prompt shows in its place: zod's `.meta({id: name})`. */
        public fun named(name: String, type: PropType): PropType =
            PropType("named", mapOf("name" to JsonPrimitive(name), "type" to type.json))
    }
}

/** A field of an inline object ([PropType.objectOf]), in declaration order. */
public class FieldDefinition(
    public val name: String,
    public val type: PropType,
    public val optional: Boolean = false,
) {
    internal val json: JsonObject = buildJsonObject {
        put("name", name)
        put("type", type.json)
        if (optional) put("optional", true)
    }
}

/** One positional argument of a component, in declaration order. Only a prop can be a binding. */
public class PropDefinition(
    public val name: String,
    public val type: PropType,
    public val optional: Boolean = false,
    /** The prop takes a `$variable` and writes it back: upstream's `$binding<type>`. */
    public val binding: Boolean = false,
) {
    internal val json: JsonObject = buildJsonObject {
        put("name", name)
        put("type", type.json)
        if (optional) put("optional", true)
        if (binding) put("binding", true)
    }
}

/** A named group of components for the prompt, with notes shown under it. */
public class ComponentGroup(
    public val name: String,
    public val components: List<String>,
    public val notes: List<String>? = null,
) {
    internal val json: JsonObject = buildJsonObject {
        put("name", name)
        put("components", JsonArray(components.map(::JsonPrimitive)))
        notes?.let { put("notes", JsonArray(it.map(::JsonPrimitive))) }
    }
}

/** A component declared for a library: its name, what it is for, its props, and what draws it. */
public class DefinedComponent internal constructor(
    public val name: String,
    /** The composable that draws the component's evaluated element; none leaves it to `ElementView`. */
    internal val content: (@Composable (Element) -> Unit)?,
    internal val json: JsonObject,
)

/**
 * Declares a component, as upstream's `defineComponent` does: a name, a
 * description for the prompt, its positional props, and the composable
 * that draws its evaluated element. `ElementView` draws an element of this
 * component with [content], given the element; without it, an element is
 * drawn as one of the library's base components when it has the name
 * (`TextContent`), and otherwise as a labelled box.
 */
public fun defineComponent(
    name: String,
    description: String,
    props: List<PropDefinition>,
    content: (@Composable (element: Element) -> Unit)? = null,
): DefinedComponent =
    DefinedComponent(name, content, buildJsonObject {
        put("name", name)
        put("description", description)
        put("props", JsonArray(props.map { it.json }))
    })

/** A component library the core built from a library definition, shared by every live program on it. */
public class Library internal constructor(
    internal val inner: ComponentLibrary,
    /** The root component the library declared; with no `root` statement, a program's entry is its first call of it. */
    internal val root: String?,
    components: List<DefinedComponent>,
) {
    /** What draws each component, by name. As in the core's library, the last declaration of a name wins. */
    internal val drawings: Map<String, (@Composable (Element) -> Unit)?> = components.associate { it.name to it.content }

    /**
     * The library's JSON Schema, as JSON text, byte for byte as upstream's
     * `JSON.stringify(library.toJSONSchema())`.
     */
    public val jsonSchema: String
        get() = checkNotNull(core { inner.jsonSchema() }) { "a library made from a definition has a schema" }

    /**
     * The library's prompt spec, as JSON text, byte for byte as upstream's
     * `JSON.stringify(library.toSpec())`.
     */
    public val promptSpec: String
        get() = checkNotNull(core { inner.promptSpec() }) { "a library made from a definition has a spec" }
}

/** The library of the live program being drawn, for `ElementView` to find a component's composable. */
internal val LocalLibrary = staticCompositionLocalOf<Library?> { null }

/**
 * Builds a library from `components`, as upstream's `createLibrary` does:
 * the core turns the definition into the library's JSON Schema and prompt
 * spec. `groups` order the components in the prompt; `root` is the
 * library's root component, which the prompt names and which a program
 * with no `root` statement starts from.
 *
 * The core refuses a definition it can't use, with an [OpenUIException]
 * carrying its kind and message: [OpenUIException.Kind.InvalidJson] naming
 * what is wrong (a `root` that isn't one of the components, say, in
 * upstream's words), or [OpenUIException.Kind.LimitExceeded] for a
 * definition longer than the core's host input limit.
 */
public fun createLibrary(
    components: List<DefinedComponent>,
    groups: List<ComponentGroup>? = null,
    root: String? = null,
): Library {
    val definition = buildJsonObject {
        put("components", JsonArray(components.map { it.json }))
        groups?.let { put("componentGroups", JsonArray(it.map { group -> group.json })) }
        root?.let { put("root", it) }
    }
    return Library(core { ComponentLibrary.fromDefinition(jsonText(definition)) }, root, components)
}
