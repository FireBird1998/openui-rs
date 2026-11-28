package dev.openui.compose

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * The core's host input limit: its default `RuntimeLimits.max_json_bytes`
 * (8 MiB, `RuntimeLimits::default` in crates/openui-ffi/src/runtime.rs), the
 * most host input text, in UTF-8 bytes, that the core admits from a session
 * or library made with the default limits. It is the core's limit, not this
 * library's: [jsonText] applies it early, while writing, so a vast text is
 * never built, and refuses with exactly what the core gives for the same
 * text. The binding doesn't expose the default, so it is restated here, as
 * OpenUINative's `defaultHostByteLimit` restates it; a JVM test sends text
 * one byte over to the core directly and fails if the core's value changes.
 */
internal const val maxJsonBytes: Long = 8L * 1024 * 1024

/**
 * [json] as the compact JSON text the core reads: what kotlinx's
 * `toString()` writes (keys in order, no spaces, its string escapes,
 * number literals as they are), with three differences.
 *
 * - It writes with a stack of the containers still open, each one's own
 *   iterator, rather than by recursing per nesting level, so a value nested
 *   100,000 deep doesn't overflow the stack, and a container's children are
 *   reached one at a time, never copied or listed ahead. A tree that shares
 *   a part writes it each time it appears, and a container or string can
 *   stand for far more text than it holds (a virtual list, escapes six bytes
 *   long), so the text's UTF-8 length is counted as each piece is written,
 *   inside strings too: writing stops as soon as it passes the core's host
 *   input limit ([maxJsonBytes]), with the core's own refusal
 *   ([OpenUIException.Kind.LimitExceeded], "host input byte limit").
 * - A non-string value is written only if its text is a JSON literal
 *   (`true`, `false`, `null` or a number by JSON's grammar). kotlinx writes
 *   any other text unchecked (a `Number` subclass's `toString()`, an
 *   unquoted literal), which could add keys to the object around it; that
 *   is refused with [OpenUIException.Kind.Decode], naming the text's start.
 * - A non-finite number is written as `null`, as JavaScript's
 *   `JSON.stringify` writes it ([nonFinite]).
 */
internal fun jsonText(json: JsonElement): String {
    val out = CountedText()
    // One frame per open container, holding the container's own iterator:
    // the stack grows with nesting depth, never with width, and no child is
    // reached before everything ahead of it is written.
    val open = ArrayDeque<Children>()
    var value: JsonElement? = json
    while (true) {
        if (value == null) {
            val frame = open.lastOrNull() ?: return out.toString()
            if (!frame.hasNext()) {
                open.removeLast()
                out.ascii(frame.close)
                continue
            }
            val child = frame.next()
            if (!frame.first) out.ascii(",")
            value = if (child is Map.Entry<*, *>) {
                out.quoted(child.key as String)
                out.ascii(":")
                child.value as JsonElement
            } else {
                child as JsonElement
            }
        }
        when (val written: JsonElement = value) {
            is JsonObject -> {
                out.ascii("{")
                open.addLast(Children(written.entries.iterator(), "}"))
            }
            is JsonArray -> {
                out.ascii("[")
                open.addLast(Children(written.iterator(), "]"))
            }
            is JsonPrimitive -> if (written.isString) out.quoted(written.content) else out.ascii(literal(written.content))
        }
        value = null
    }
}

/** An open container: its children (an object's entries in order, or an array's items), and the text that closes it. */
private class Children(private val inner: Iterator<*>, val close: String) {
    private var started = false

    /** The child [next] gave last is the container's first. */
    var first: Boolean = false
        private set

    fun hasNext(): Boolean = inner.hasNext()

    fun next(): Any? {
        first = !started
        started = true
        return inner.next()
    }
}

/**
 * The text being written, and its length in UTF-8, counted as it grows: past
 * the core's host input limit it stops with the core's own refusal, before
 * the rest is written. A lone surrogate counts as the three bytes of U+FFFD.
 */
private class CountedText {
    private val text = StringBuilder()
    private var bytes = 0L

    private fun count(size: Int) {
        bytes += size
        if (bytes > maxJsonBytes) throw OpenUIException(OpenUIException.Kind.LimitExceeded, "host input byte limit")
    }

    /** Text known to be ASCII: punctuation, escapes and literals. */
    fun ascii(piece: String) {
        count(piece.length)
        text.append(piece)
    }

    /** A string as kotlinx writes it: `"`, `\` and the C0 controls escaped, the rest as it is. */
    fun quoted(value: String) {
        ascii("\"")
        var i = 0
        while (i < value.length) {
            val c = value[i]
            when {
                c == '"' -> ascii("\\\"")
                c == '\\' -> ascii("\\\\")
                c == '\t' -> ascii("\\t")
                c == '\b' -> ascii("\\b")
                c == '\n' -> ascii("\\n")
                c == '\r' -> ascii("\\r")
                c == '\u000C' -> ascii("\\f")
                c < ' ' -> {
                    count(6)
                    text.append("\\u00").append(HEX[c.code shr 4]).append(HEX[c.code and 15])
                }
                c.code < 0x80 -> { count(1); text.append(c) }
                c.code < 0x800 -> { count(2); text.append(c) }
                c.isHighSurrogate() && i + 1 < value.length && value[i + 1].isLowSurrogate() -> {
                    count(4)
                    text.append(c).append(value[i + 1])
                    i++
                }
                else -> { count(3); text.append(c) }
            }
            i++
        }
        ascii("\"")
    }

    override fun toString(): String = text.toString()

    private companion object {
        const val HEX = "0123456789abcdef"
    }
}

/** A non-string value's text, if it is a JSON literal; otherwise the library's refusal. */
private fun literal(text: String): String {
    if (text == "true" || text == "false" || text == "null" || isJsonNumber(text)) return text
    nonFinite(text)?.let { return it }
    val start = if (text.length <= 32) text else text.take(32) + "… (shortened; ${text.length} characters in all)"
    throw OpenUIException(OpenUIException.Kind.Decode, "not a JSON literal: $start")
}

/**
 * A non-finite number, as kotlinx writes a Double's or Float's (`NaN`,
 * `Infinity`, `-Infinity`), becomes `null`, as `JSON.stringify` makes it:
 * the JSON boundary rule, ruled by the product manager for #259. kotlinx
 * keeps only a value's text, so a `Number` subclass or unquoted literal
 * whose text is one of these words can't be told from a real one, and is
 * written as `null` too.
 */
private fun nonFinite(text: String): String? =
    if (text == "NaN" || text == "Infinity" || text == "-Infinity") "null" else null

/** Whether [text] is a number by JSON's grammar: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`, scanned once. */
private fun isJsonNumber(text: String): Boolean {
    var i = 0
    val n = text.length
    fun digits(): Boolean {
        val start = i
        while (i < n && text[i] in '0'..'9') i++
        return i > start
    }
    if (i < n && text[i] == '-') i++
    if (i < n && text[i] == '0') i++ else if (i >= n || text[i] !in '1'..'9' || !digits()) return false
    if (i < n && text[i] == '.') {
        i++
        if (!digits()) return false
    }
    if (i < n && (text[i] == 'e' || text[i] == 'E')) {
        i++
        if (i < n && (text[i] == '+' || text[i] == '-')) i++
        if (!digits()) return false
    }
    return i == n
}
