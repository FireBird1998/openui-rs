package dev.openui.demo

import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.round

/** A value in a parse result: an element, a list, a record, an expression a
 * runtime would evaluate, or a plain JSON value. */
sealed interface PV {
    data class El(val typeName: String, val props: Map<String, PV>, val partial: Boolean) : PV
    data class Lst(val items: List<PV>) : PV
    data class Rec(val entries: Map<String, PV>) : PV
    data class Expr(val source: String) : PV
    data class Str(val s: String) : PV
    data class Num(val n: Double) : PV
    data class Bool(val b: Boolean) : PV
    data object Null : PV
}

/** The `k` of an expression node in a result. */
private val expressionKinds = setOf(
    "Arr", "Assign", "BinOp", "Bool", "Comp", "Index", "Member", "Null", "Num", "Obj",
    "Ph", "Ref", "RuntimeRef", "StateRef", "Str", "Ternary", "UnaryOp",
)

/** The result's root element, from the JSON the core writes. */
fun rootOf(json: String): PV? = when (val root = JSONObject(json).opt("root")) {
    null, JSONObject.NULL -> null
    else -> toPV(root)
}

fun toPV(v: Any?): PV = when (v) {
    null, JSONObject.NULL -> PV.Null
    is JSONObject -> when {
        v.optString("type") == "element" && v.has("typeName") -> {
            val props = v.optJSONObject("props")
            PV.El(
                v.getString("typeName"),
                props?.keys()?.asSequence()?.associateWith { toPV(props.opt(it)) } ?: emptyMap(),
                v.optBoolean("partial"),
            )
        }
        v.optString("k") in expressionKinds -> PV.Expr(summary(v))
        else -> PV.Rec(v.keys().asSequence().associateWith { toPV(v.opt(it)) })
    }
    is JSONArray -> PV.Lst((0 until v.length()).map { toPV(v.opt(it)) })
    is String -> PV.Str(v)
    is Number -> PV.Num(v.toDouble())
    is Boolean -> PV.Bool(v)
    else -> PV.Str(v.toString())
}

/** A short form of an expression, shown where a runtime would put its value. */
private fun summary(e: JSONObject): String = when (e.optString("k")) {
    "StateRef" -> "$" + e.optString("n")
    "Ref", "RuntimeRef" -> e.optString("n")
    else -> "‹" + e.optString("k") + "›"
}

fun countElements(v: PV?): Int = when (v) {
    is PV.El -> 1 + v.props.values.sumOf { countElements(it) }
    is PV.Lst -> v.items.sumOf { countElements(it) }
    is PV.Rec -> v.entries.values.sumOf { countElements(it) }
    else -> 0
}

fun fmt(n: Double): String =
    if (n == round(n) && abs(n) < 1e15) n.toLong().toString() else n.toString()

/** Reads an element's props; an absent or null prop reads as missing. */
class Props(val el: PV.El) {
    fun v(name: String): PV? = el.props[name]?.takeUnless { it is PV.Null }

    fun text(name: String): String? = when (val x = v(name)) {
        is PV.Str -> x.s
        is PV.Num -> fmt(x.n)
        is PV.Bool -> x.b.toString()
        is PV.Expr -> x.source
        else -> null
    }

    fun word(name: String, fallback: String) = (v(name) as? PV.Str)?.s ?: fallback
    fun bool(name: String) = (v(name) as? PV.Bool)?.b ?: false
    fun num(name: String) = (v(name) as? PV.Num)?.n
    fun list(name: String): List<PV> = (v(name) as? PV.Lst)?.items ?: emptyList()
    fun elements(name: String, type: String) =
        list(name).filterIsInstance<PV.El>().filter { it.typeName == type }
    fun element(name: String) = v(name) as? PV.El
}
