package dev.openui.compose

import dev.openui.compose.ffi.ComponentLibrary
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

/** A prop's type in a library definition (ADR 0017): the core reads it. */
public class PropType private constructor(internal val json: JsonObject) {
    public companion object {
        public val string: PropType = PropType(buildJsonObject { put("kind", "string") })
    }
}

/** One positional argument of a component, in declaration order. */
public class PropDefinition(
    public val name: String,
    public val type: PropType,
    public val optional: Boolean = false,
) {
    internal val json: JsonObject = buildJsonObject {
        put("name", name)
        put("type", type.json)
        if (optional) put("optional", JsonPrimitive(true))
    }
}

/** A component declared for a library: its name, what it is for, and its props. */
public class DefinedComponent internal constructor(
    public val name: String,
    internal val json: JsonObject,
)

/**
 * Declares a component. The library draws it as one of its base components
 * when it knows the name (`TextContent`), and otherwise as a labelled box.
 */
public fun defineComponent(name: String, description: String, props: List<PropDefinition>): DefinedComponent =
    DefinedComponent(name, buildJsonObject {
        put("name", name)
        put("description", description)
        putJsonArray("props") { props.forEach { add(it.json) } }
    })

/** A component library the core built from a library definition, shared by every live program on it. */
public class Library internal constructor(internal val inner: ComponentLibrary)

/**
 * Builds a library from `components`. The core turns the definition into
 * the library's JSON Schema, as upstream's `createLibrary` does; it refuses
 * a definition it can't use, with the core's error kind.
 */
public fun createLibrary(components: List<DefinedComponent>): Library {
    val definition = buildJsonObject {
        putJsonArray("components") { components.forEach { add(it.json) } }
    }
    return Library(core { ComponentLibrary.fromDefinition(definition.toString()) })
}
