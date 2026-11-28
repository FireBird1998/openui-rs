package dev.openui.compose

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull

/** One component instance in the evaluated tree, with its positional arguments mapped to named props. */
public class Element internal constructor(
    public val typeName: String,
    /** The props in the order the core wrote them. */
    public val props: List<Prop>,
    /** Still arriving: the statement or one of its parts isn't complete yet. */
    public val partial: Boolean,
    public val hasDynamicProps: Boolean,
    public val statementId: String?,
) {
    /** The value of the prop called [name], or null when it has none. */
    public fun prop(name: String): PropValue? = props.firstOrNull { it.name == name }?.value

    override fun toString(): String = "Element($typeName, $props)"
}

public data class Prop(public val name: String, public val value: PropValue)

/**
 * A prop's value. Decoding is lossless: every JSON value the core writes
 * becomes one of these. Binding markers, action plans and expressions are
 * told apart in a later ticket (#260); until then they arrive as records.
 */
public sealed interface PropValue {
    public data class ElementValue(public val element: Element) : PropValue
    public data class ListValue(public val items: List<PropValue>) : PropValue
    /** A plain object; its fields may hold elements too. */
    public data class Record(public val fields: List<Prop>) : PropValue
    /** A string, number, boolean or null, exactly as the core wrote it. */
    public data class Scalar(public val json: JsonPrimitive) : PropValue
}

/** The keys of an element node, in the order the core writes them; `statementId` may follow. */
private val elementKeys = listOf("type", "typeName", "props", "partial", "hasDynamicProps")

/** The element at [json] if it is one, in exactly the core's shape (as OpenUINative reads it). */
internal fun elementOrNull(json: JsonElement): Element? {
    if (json !is JsonObject || (json.size != 5 && json.size != 6)) return null
    val keys = json.keys.toList()
    if (keys.take(5) != elementKeys || (json.size == 6 && keys[5] != "statementId")) return null
    val type = json["type"] as? JsonPrimitive
    val typeName = json["typeName"] as? JsonPrimitive
    val props = json["props"] as? JsonObject
    val partial = (json["partial"] as? JsonPrimitive)?.takeUnless { it.isString }?.booleanOrNull
    val dynamic = (json["hasDynamicProps"] as? JsonPrimitive)?.takeUnless { it.isString }?.booleanOrNull
    val statementId = json["statementId"]?.let { it as? JsonPrimitive }
    if (type?.isString != true || type.content != "element" || typeName?.isString != true || props == null ||
        partial == null || dynamic == null || (json.size == 6 && statementId?.isString != true)
    ) return null
    return Element(
        typeName = typeName.content,
        props = props.map { (name, value) -> Prop(name, propValue(value)) },
        partial = partial,
        hasDynamicProps = dynamic,
        statementId = statementId?.content,
    )
}

internal fun propValue(json: JsonElement): PropValue = when (json) {
    is JsonPrimitive -> PropValue.Scalar(json)
    is JsonArray -> PropValue.ListValue(json.map(::propValue))
    is JsonObject -> elementOrNull(json)?.let(PropValue::ElementValue)
        ?: PropValue.Record(json.map { (name, value) -> Prop(name, propValue(value)) })
}
