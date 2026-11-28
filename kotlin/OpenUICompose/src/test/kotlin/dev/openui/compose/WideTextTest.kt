package dev.openui.compose

import java.util.Collections
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * Values that are wide rather than deep, and strings whose escaped text is
 * vast, are refused at the core's host input limit after writing about that
 * much: the library never expands a container's children or a string's
 * escapes ahead of the limit. What a call allocates is read from the JVM's
 * own accounting for the calling thread (not a clock), and must stay under
 * [ceiling]: eight times the limit, room for the output text (at most the
 * limit, held as UTF-16 at twice its size, and grown by doubling, so at
 * most about four times it) and the refusal. Expanding a 100,000,000-wide
 * array's children, or a string's 6-byte escapes, takes far more.
 */
class WideTextTest {
    private val ceiling = 8 * maxJsonBytes

    /** The bytes this thread allocates during [call], from the JVM's ThreadMXBean (by reflection: the unit tests compile against android.jar). */
    private fun allocatedDuring(call: () -> Unit): Long {
        val bean = Class.forName("java.lang.management.ManagementFactory").getMethod("getThreadMXBean").invoke(null)
        val type = Class.forName("com.sun.management.ThreadMXBean")
        assertTrue("the JVM counts what a thread allocates",
            type.getMethod("isThreadAllocatedMemorySupported").invoke(bean) as Boolean &&
                type.getMethod("isThreadAllocatedMemoryEnabled").invoke(bean) as Boolean)
        val read = type.getMethod("getCurrentThreadAllocatedBytes")
        val before = read.invoke(bean) as Long
        call()
        return read.invoke(bean) as Long - before
    }

    private val library = createLibrary(listOf(defineComponent("A", "d", listOf(PropDefinition("v", PropType.string)))))

    private fun tool(schema: JsonElement) = PromptOptions(tools = listOf(PromptTool.Spec(ToolSpec("lookup", inputSchema = schema))))

    /** [call] is refused at the host input limit, allocating less than [ceiling]. */
    private fun refusedAtTheLimitCheaply(call: () -> Unit) {
        var error: OpenUIException? = null
        val allocated = allocatedDuring {
            try {
                call()
            } catch (refusal: OpenUIException) {
                error = refusal
            }
        }
        val refusal = error ?: run { fail("expected a refusal"); return }
        assertEquals(OpenUIException.Kind.LimitExceeded, refusal.kind)
        assertEquals("host input byte limit", refusal.message)
        println("allocated $allocated bytes")
        assertTrue("allocated $allocated bytes, over $ceiling", allocated < ceiling)
    }

    @Test
    fun aHundredMillionWideArrayIsRefusedAfterWritingAboutTheLimit() {
        val wide = Collections.nCopies(100_000_000, JsonPrimitive(1))
        assertEquals(100_000_000, JsonArray(wide).size)
        refusedAtTheLimitCheaply { systemPrompt(library, tool(JsonObject(mapOf("enum" to JsonArray(wide))))) }
    }

    @Test
    fun anArrayAsWideAsAListCanBeIsRefusedAfterWritingAboutTheLimit() {
        val wide = JsonArray(Collections.nCopies(Int.MAX_VALUE, JsonPrimitive(1)))
        assertEquals(Int.MAX_VALUE, wide.size)
        refusedAtTheLimitCheaply { systemPrompt(library, tool(wide)) }
    }

    @Test
    fun aHundredMillionWideObjectIsRefusedAfterWritingAboutTheLimit() {
        // A map that stands for 100,000,000 entries and makes none: its
        // iterator hands out one entry again and again.
        val entry = java.util.AbstractMap.SimpleImmutableEntry<String, JsonElement>("k", JsonPrimitive(1))
        val wide = object : java.util.AbstractMap<String, JsonElement>() {
            override val entries: MutableSet<MutableMap.MutableEntry<String, JsonElement>> =
                object : java.util.AbstractSet<MutableMap.MutableEntry<String, JsonElement>>() {
                    override val size = 100_000_000
                    override fun iterator() = object : MutableIterator<MutableMap.MutableEntry<String, JsonElement>> {
                        var left = size
                        override fun hasNext() = left > 0
                        override fun next(): MutableMap.MutableEntry<String, JsonElement> = entry.also { left-- }
                        override fun remove() = throw UnsupportedOperationException()
                    }
                }
        }
        assertEquals(100_000_000, JsonObject(wide).size)
        refusedAtTheLimitCheaply { systemPrompt(library, tool(JsonObject(wide))) }
    }

    @Test
    fun controlCharactersPastTheLimitWhenEscapedAreRefusedWithoutEscapingThemAll() {
        // 20,000,000 characters, each written as six (\u0001): 120 MB of
        // text, past the limit long before the end.
        val controls = "\u0001".repeat(20_000_000)
        assertTrue(6L * controls.length > maxJsonBytes)
        refusedAtTheLimitCheaply { systemPrompt(library, PromptOptions(preamble = controls)) }
        refusedAtTheLimitCheaply { createLibrary(listOf(defineComponent("A", controls, emptyList()))) }
    }

    @Test
    fun rawControlCharactersAreEscapedSoTheCoreTakesThemAsWritten() {
        val description = "a\u0001b\u001Fc\u0000d\u007F"
        val spec = Json.parseToJsonElement(createLibrary(listOf(defineComponent("A", description, emptyList()))).promptSpec)
        assertEquals(description, spec.jsonObject.getValue("components").jsonObject.getValue("A").jsonObject
            .getValue("description").jsonPrimitive.content)
        // In a tool's schema too: the enum value reaches the prompt as given.
        val schema = Json.parseToJsonElement("""{"type":"object","properties":{"q":{"type":"string"}}}""").jsonObject
        val withEnum = JsonObject(schema + ("properties" to JsonObject(mapOf("q" to JsonObject(mapOf(
            "type" to JsonPrimitive("string"), "enum" to JsonArray(listOf(JsonPrimitive("x\u0001y")))))))))
        assertTrue(systemPrompt(library, tool(withEnum)).contains("q?: \"x\u0001y\""))
    }

    @Test
    fun aSchemasKeysReachThePromptInTheCallersOrder() {
        // Upstream lists a tool's arguments in its schema's own key order
        // (reference/lang-core/src/parser/prompt.ts, renderToolSignature:
        // Object.entries(props), lines 477-487), which for keys that aren't
        // array indices is insertion order. No recorded fixture has keys
        // out of alphabetical order; this expectation comes from reading
        // upstream.
        val schema = JsonObject(mapOf("type" to JsonPrimitive("object"), "properties" to JsonObject(mapOf(
            "zeta" to JsonObject(mapOf("type" to JsonPrimitive("string"))),
            "alpha" to JsonObject(mapOf("type" to JsonPrimitive("number")))))))
        assertTrue(systemPrompt(library, tool(schema)).contains("- lookup(zeta?: string, alpha?: number)\n"))
    }
}
