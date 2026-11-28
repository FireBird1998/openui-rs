package dev.openui.compose

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * Declarations whose text is deep or vast reach the core as its typed
 * refusals, never as a StackOverflowError or an OutOfMemoryError: the
 * library writes its definition and options without recursing per nesting
 * level, and stops writing as soon as the text passes the core's host
 * input limit. The deep cases also run on a thread with a small stack, as a
 * device's threads have less than the test JVM's.
 */
class DeepDefinitionTest {
    // The core's limit on how deep a definition's types nest; host JSON
    // itself is refused past 800 levels.
    private val coreDepthLimit = 256
    private val deep = 100_000

    private fun refusal(call: () -> Unit): OpenUIException {
        try {
            call()
        } catch (error: OpenUIException) {
            return error
        }
        fail("expected a refusal")
        throw AssertionError()
    }

    /** [call] on this thread and on one with a 256 KiB stack: the same refusal both times. */
    private fun refusedOnBothStacks(call: () -> Unit): OpenUIException {
        val here = refusal(call)
        var there: Throwable? = null
        val small = Thread(null, { there = runCatching { refusal(call) }.fold({ it }, { it }) }, "small-stack", 256L * 1024)
        small.start()
        small.join()
        val onSmall = there
        if (onSmall !is OpenUIException) throw AssertionError("on a small stack: $onSmall", onSmall)
        assertEquals(here.kind, onSmall.kind)
        assertEquals(here.message, onSmall.message)
        return here
    }

    private fun library(type: PropType) = createLibrary(listOf(defineComponent("A", "d", listOf(PropDefinition("v", type)))))

    @Test
    fun anArrayTypeNested100000DeepIsTheCoresDepthRefusal() {
        var type = PropType.string
        repeat(deep) { type = PropType.array(type) }
        // Past the core's depth limit, and short enough for the core to
        // read it: 25 bytes a level, about 2.4 MiB.
        assertTrue(deep > coreDepthLimit)
        assertTrue(deep * """{"kind":"array","items":}""".length + 100 < maxJsonBytes)
        val error = refusedOnBothStacks { library(type) }
        assertEquals(error.message, OpenUIException.Kind.InvalidJson, error.kind)
    }

    @Test
    fun anObjectTypeNested100000DeepIsTheCoresDepthRefusal() {
        var type = PropType.string
        repeat(deep) { type = PropType.objectOf(listOf(FieldDefinition("f", type))) }
        assertTrue(deep * """{"kind":"object","fields":[{"name":"f","type":}]}""".length + 100 < maxJsonBytes)
        val error = refusedOnBothStacks { library(type) }
        assertEquals(error.message, OpenUIException.Kind.InvalidJson, error.kind)
    }

    @Test
    fun aUnionSharingOneTypeThirtyTimesIsRefusedAtTheHostInputLimitWithoutWritingItAll() {
        var type = PropType.string
        repeat(30) { type = PropType.union(listOf(type, type)) }
        // Written whole it would hold 2^30 copies of {"kind":"string"}: 17
        // GiB, far past the test JVM's 512 MiB heap, so a refusal here means
        // writing stopped at the limit.
        assertTrue((1L shl 30) * """{"kind":"string"}""".length > 16L * 1024 * 1024 * 1024)
        val error = refusal { library(type) }
        assertEquals(OpenUIException.Kind.LimitExceeded, error.kind)
        assertEquals("host input byte limit", error.message)
    }

    @Test
    fun aToolSchemaNested100000DeepIsRefusedTyped() {
        var schema: JsonElement = JsonPrimitive(1)
        repeat(deep) { schema = JsonArray(listOf(schema)) }
        assertTrue(2L * deep + 1000 < maxJsonBytes)
        val library = library(PropType.string)
        val options = PromptOptions(tools = listOf(PromptTool.Spec(ToolSpec("t", inputSchema = JsonObject(mapOf("items" to schema))))))
        val error = refusedOnBothStacks { systemPrompt(library, options) }
        assertEquals(error.message, OpenUIException.Kind.InvalidPromptSpec, error.kind)
    }

    @Test
    fun theLibrarysHostInputLimitIsTheCoresOwn() {
        // The value the library stops writing at is the core's default
        // max_json_bytes: the core itself admits Cloud instructions of
        // exactly that many bytes and refuses one more, with the message
        // the library's own refusal uses.
        assertEquals(8L * 1024 * 1024, maxJsonBytes)
        val library = library(PropType.string)
        val exact = "a".repeat(maxJsonBytes.toInt())
        assertTrue(cloudConfigBlock(library, instructions = exact).endsWith(exact))
        val error = refusal { cloudConfigBlock(library, instructions = exact + "a") }
        assertEquals(OpenUIException.Kind.LimitExceeded, error.kind)
        assertEquals("host input byte limit", error.message)
    }

    @Test
    fun atTheBoundaryTheWriterAndTheCoreAgree() {
        // A definition one byte under the limit and one at it reach the core
        // and are accepted; one byte over is refused by the writer, and the
        // same text sent to the core directly, past the writer, is refused
        // by the core in the same words. If the core's value changes, one of
        // these fails.
        val prefix = """{"components":[{"name":"A","description":""""
        val suffix = """","props":[]}]}"""
        val text = { bytes: Long -> prefix + "a".repeat((bytes - prefix.length - suffix.length).toInt()) + suffix }
        val description = { bytes: Long -> "a".repeat((bytes - prefix.length - suffix.length).toInt()) }
        for (bytes in listOf(maxJsonBytes - 1, maxJsonBytes)) {
            assertEquals(bytes, text(bytes).encodeToByteArray().size.toLong())
            val library = createLibrary(listOf(defineComponent("A", description(bytes), emptyList())))
            assertTrue(library.promptSpec.contains("a".repeat(1000)))
        }
        val over = maxJsonBytes + 1
        assertEquals(over, text(over).encodeToByteArray().size.toLong())
        val byWriter = refusal { createLibrary(listOf(defineComponent("A", description(over), emptyList()))) }
        val byCore = refusal { core { dev.openui.compose.ffi.ComponentLibrary.fromDefinition(text(over)) } }
        assertEquals(OpenUIException.Kind.LimitExceeded, byCore.kind)
        assertEquals("host input byte limit", byCore.message)
        assertEquals(byCore.kind, byWriter.kind)
        assertEquals(byCore.message, byWriter.message)
    }
}
