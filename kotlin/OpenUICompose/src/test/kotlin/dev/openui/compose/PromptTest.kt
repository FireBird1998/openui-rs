package dev.openui.compose

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The system prompt and the Cloud config block of libraries defined in
 * Kotlin, against upstream's own output: every recorded prompt of a
 * recorded definition the typed declarations can say, with options the
 * typed options can say, is asked for and must be the recorded text byte
 * for byte, or the recorded refusal. The recording is
 * conformance/golden/library-definition-prompts.json, written by
 * reference/library-definitions.ts from upstream's `library.prompt(options)`
 * and `generateSystemPrompt({cloud: true, library: library.toSpec(), ...})`.
 */
class PromptTest {
    private val golden = Recorded.golden("library-definition-prompts.json").jsonObject

    /** The typed library for each recorded definition the typed declarations can say. */
    private val libraries: Map<String, Library> =
        Recorded.golden("library-definitions.json").jsonArray
            .filter { it.field("error") == null }
            .mapNotNull { case ->
                try {
                    case.name() to typedDefinition(case.jsonObject.getValue("definition")).create()
                } catch (_: Untyped) {
                    null
                }
            }
            .toMap()

    @Test
    fun theFullPromptIsUpstreamsForEveryRecordedTypedLibraryAndOptions() {
        assertEquals("libraries the typed declarations can say", 51, libraries.size)
        val cases = golden.getValue("prompts").jsonArray
        assertEquals("the recording's prompts changed", 131, cases.size)
        var asked = 0
        val withOptions = mutableSetOf<String>()
        for (case in cases) {
            val library = libraries[case.name()] ?: continue
            val options = try {
                typedOptions(case.field("options"))
            } catch (_: Untyped) {
                continue
            }
            val want = case.field("prompt")!!.text!!
            assertEquals("${case.name()} ${case.field("options")}", want, systemPrompt(library, options))
            asked++
            case.field("options")?.let { withOptions += (it as JsonObject).keys }
        }
        // All but three: upstream's spread lets `{root, componentGroups}`
        // through library.prompt, which isn't one of its PromptOptions.
        assertEquals("prompts asked for", 75, asked)
        assertEquals("every option was given", setOf("tools", "editMode", "inlineMode", "toolCalls", "bindings",
            "builtinFunctions", "preamble", "examples", "toolExamples", "additionalRules"), withOptions)
    }

    @Test
    fun theCloudConfigBlockIsUpstreamsForEveryRecordedTypedLibrary() {
        val cases = golden.getValue("cloud").jsonArray
        assertEquals("the recording's cloud configs changed", 88, cases.size)
        var asked = 0
        var refusals = 0
        for (case in cases) {
            val library = libraries[case.name()] ?: continue
            // Upstream picks the options Cloud carries from whatever it is
            // given (here tools and editMode too); so does CloudPromptOptions(options).
            val options = CloudPromptOptions(typedOptions(case.field("promptOptions")))
            val instructions = case.field("instructions")?.text
            val where = "${case.name()} ${case.field("promptOptions")} $instructions"
            val error = case.field("error")?.text
            if (error == null) {
                assertEquals(where, case.field("config")!!.text, cloudConfigBlock(library, options, instructions))
            } else {
                try {
                    cloudConfigBlock(library, options, instructions)
                    throw AssertionError("$where: expected upstream's refusal: $error")
                } catch (refusal: OpenUIException) {
                    assertEquals(where, OpenUIException.Kind.Prompt, refusal.kind)
                    assertEquals(where, error, refusal.message)
                }
                refusals++
            }
            asked++
        }
        assertEquals("cloud configs asked for", 60, asked)
        assertEquals("refusals asked for", 11, refusals)
    }

    @Test
    fun cloudOptionsArePickedFromTheFullOnesAsUpstreamPicksThem() {
        val everything = PromptOptions(
            tools = listOf(PromptTool.Name("search")), editMode = true, inlineMode = true, toolCalls = true,
            bindings = true, builtinFunctions = true, preamble = "Acme.", examples = listOf("root = Page(\"x\", [])"),
            toolExamples = listOf("q = Query(\"search\", {}, [])"), additionalRules = listOf("No red."),
        )
        assertEquals(
            CloudPromptOptions(examples = listOf("root = Page(\"x\", [])"), preamble = "Acme.",
                additionalRules = listOf("No red.")),
            CloudPromptOptions(everything),
        )
        val library = libraries.getValue("scalars")
        val block = cloudConfigBlock(library, CloudPromptOptions(everything))
        assertTrue(block.startsWith("]]>openui:config\n"))
        assertTrue("no tool examples in a Cloud block", !block.contains("q = Query"))
        assertEquals(block, cloudConfigBlock(library, CloudPromptOptions(
            examples = listOf("root = Page(\"x\", [])"), preamble = "Acme.", additionalRules = listOf("No red."))))
    }
}
