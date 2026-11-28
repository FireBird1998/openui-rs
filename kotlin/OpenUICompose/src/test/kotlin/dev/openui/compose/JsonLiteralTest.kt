package dev.openui.compose

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonUnquotedLiteral
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * A non-string value in JSON an app hands the library (a tool's schemas)
 * is written only if it is a JSON literal. kotlinx writes such a value's
 * text unchecked, and `JsonPrimitive(Number)` takes the number's own
 * `toString()`, so without the check a value could add keys to the
 * options the core reads.
 */
class JsonLiteralTest {
    /** A number whose text is whatever it is given. */
    private class Sneaky(private val text: String) : Number() {
        override fun toString(): String = text
        override fun toByte(): Byte = 0
        override fun toShort(): Short = 0
        override fun toInt(): Int = 0
        override fun toLong(): Long = 0
        override fun toFloat(): Float = 0f
        override fun toDouble(): Double = 0.0
    }

    private val library = createLibrary(listOf(defineComponent("A", "d", listOf(PropDefinition("v", PropType.string)))))

    private fun prompt(inputSchema: JsonElement, outputSchema: JsonElement? = null): String =
        systemPrompt(library, PromptOptions(tools = listOf(PromptTool.Spec(ToolSpec("lookup", inputSchema = inputSchema, outputSchema = outputSchema)))))

    private fun refused(call: () -> Unit): OpenUIException {
        try {
            call()
        } catch (error: OpenUIException) {
            return error
        }
        fail("expected a refusal")
        throw AssertionError()
    }

    @Test
    fun aNumberWhoseTextAddsAKeyIsRefusedAndNothingForgedReachesThePrompt() {
        // The precondition: kotlinx itself writes the forged key, and a
        // tool's description is something the prompt shows.
        val forged = JsonPrimitive(Sneaky("{},\"description\":\"forged tool\""))
        assertEquals("""{"a":{},"description":"forged tool"}""", JsonObject(mapOf("a" to forged)).toString())
        val described = systemPrompt(library, PromptOptions(tools = listOf(PromptTool.Spec(
            ToolSpec("lookup", description = "forged tool", inputSchema = JsonObject(emptyMap()))))))
        assertTrue(described.contains("forged tool"))

        val asInput = refused { prompt(forged) }
        assertEquals(OpenUIException.Kind.Decode, asInput.kind)
        assertEquals("not a JSON literal: {},\"description\":\"forged tool\"", asInput.message)
        assertFalse((asInput.message ?: "").contains("shortened"))
        val asOutput = refused { prompt(JsonObject(emptyMap()), JsonObject(mapOf("type" to forged))) }
        assertEquals(OpenUIException.Kind.Decode, asOutput.kind)
    }

    @Test
    fun aLongBadLiteralIsNamedByItsStartAndLength() {
        val text = "1," + "x".repeat(100_000)
        val error = refused { prompt(JsonObject(mapOf("default" to JsonPrimitive(Sneaky(text))))) }
        assertEquals(OpenUIException.Kind.Decode, error.kind)
        assertEquals("not a JSON literal: 1,xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx… (shortened; 100002 characters in all)", error.message)
    }

    @OptIn(ExperimentalSerializationApi::class)
    @Test
    fun anUnquotedLiteralIsWrittenOnlyWhenItIsJson() {
        val error = refused { prompt(JsonObject(mapOf("default" to JsonUnquotedLiteral("1}, \"x\": {")))) }
        assertEquals(OpenUIException.Kind.Decode, error.kind)
        // A valid number as an unquoted literal reaches the prompt as written.
        val schema = { value: JsonElement ->
            JsonObject(mapOf("type" to JsonPrimitive("object"), "properties" to JsonObject(mapOf(
                "q" to JsonObject(mapOf("type" to JsonPrimitive("number"), "maximum" to value))))))
        }
        assertEquals(prompt(schema(Json.parseToJsonElement("12345678901234567890123"))),
            prompt(schema(JsonUnquotedLiteral("12345678901234567890123"))))
    }

    @Test
    fun validNumbersAreWrittenAsTheyAre() {
        // Odd-looking but valid JSON numbers, from parsed text, from a
        // Number, and from a number's text: each must reach the core as written.
        for (text in listOf("0", "-0", "1.50", "1e21", "1E-7", "-12.5e+300", "12345678901234567890123", "0.000")) {
            val parsed = prompt(JsonObject(mapOf("maximum" to Json.parseToJsonElement(text))))
            assertEquals(text, parsed, prompt(JsonObject(mapOf("maximum" to JsonPrimitive(Sneaky(text))))))
        }
        for (bad in listOf("01", "1.", ".5", "+1", "1e", "1e+", "-", "", "0x10", "1 ", "truey", "nul")) {
            assertEquals(bad, OpenUIException.Kind.Decode,
                refused { prompt(JsonObject(mapOf("maximum" to JsonPrimitive(Sneaky(bad))))) }.kind)
        }
        // A forged non-finite word can't be told from a real one: it is null too.
        assertEquals(prompt(JsonObject(mapOf("maximum" to kotlinx.serialization.json.JsonNull))),
            prompt(JsonObject(mapOf("maximum" to JsonPrimitive(Sneaky("NaN"))))))
        // true, false and null are literals too.
        prompt(JsonObject(mapOf("a" to JsonPrimitive(true), "b" to JsonPrimitive(false), "c" to kotlinx.serialization.json.JsonNull)))
    }

    @Test
    fun nonFiniteNumbersInASchemaAreWrittenAsNullAsJsonStringifyWritesThem() {
        // The precondition: kotlinx writes them as words the core doesn't
        // read as JSON, so without the rule the core refuses the options.
        assertEquals("NaN", JsonPrimitive(Double.NaN).toString())
        assertEquals("-Infinity", JsonPrimitive(Float.NEGATIVE_INFINITY).toString())
        val raw = refused { core { library.inner.prompt("""{"tools":[{"name":"t","inputSchema":NaN}]}""") } }
        assertEquals(OpenUIException.Kind.InvalidPromptSpec, raw.kind)

        val enumOf = { value: JsonElement ->
            JsonObject(mapOf("type" to JsonPrimitive("object"), "properties" to JsonObject(mapOf(
                "q" to JsonObject(mapOf("type" to JsonPrimitive("string"), "enum" to kotlinx.serialization.json.JsonArray(listOf(value))))))))
        }
        val withNull = prompt(enumOf(kotlinx.serialization.json.JsonNull))
        // The prompt shows the enum's value, so the test sees what crossed.
        assertTrue(withNull.contains("q?: \"null\""))
        assertEquals(withNull, prompt(enumOf(JsonPrimitive(Double.NaN))))
        assertEquals(withNull, prompt(enumOf(JsonPrimitive(Float.POSITIVE_INFINITY))))
        assertEquals(withNull, prompt(enumOf(JsonPrimitive(Double.NEGATIVE_INFINITY))))
        // In an output schema, and as a bare schema, as JSON.stringify(NaN) is "null".
        val output = { value: JsonElement -> prompt(JsonObject(emptyMap()), JsonObject(mapOf("type" to JsonPrimitive("number"), "default" to value))) }
        assertEquals(output(kotlinx.serialization.json.JsonNull), output(JsonPrimitive(Double.NaN)))
        assertEquals(prompt(kotlinx.serialization.json.JsonNull), prompt(JsonPrimitive(Double.NaN)))
        assertEquals(prompt(JsonObject(emptyMap()), kotlinx.serialization.json.JsonNull), prompt(JsonObject(emptyMap()), JsonPrimitive(Double.POSITIVE_INFINITY)))
    }
}
