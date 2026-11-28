package dev.openui.compose

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The live program seam on an ordinary JVM: the core built for this machine,
 * loaded through the generated binding and JNA. Nothing here can pass without
 * the core: the text below is put together by the core's evaluator, and the
 * positional argument is named by the library the core built.
 */
class LiveProgramTest {
    private val textContent = defineComponent(
        name = "TextContent",
        description = "A paragraph of text.",
        props = listOf(PropDefinition("text", PropType.string)),
    )

    @Test
    fun showsAProgramAsTheTreeTheCoreEvaluated() = runBlocking {
        val live = LiveProgram(createLibrary(listOf(textContent)))
        try {
            assertNull("nothing is shown before a program", live.root)

            live.show("greeting = \"Hello\"\nroot = TextContent(greeting + \", JVM\")")

            val root = live.root
            assertNotNull("the core evaluated a root", root)
            assertEquals("TextContent", root!!.typeName)
            assertEquals(listOf(Prop("text", PropValue.Scalar(JsonPrimitive("Hello, JVM")))), root.props)
            assertFalse(root.partial)
            assertFalse(live.sizeLimited)
            assertFalse(live.depthLimited)
        } finally {
            live.dispose()
        }
    }

    // The core's defaults (ParseLimits): a program of at most 1 MiB, and 256
    // levels of nesting. Past either the core cuts what it returns and says
    // so; a live program must pass that on, not show the cut silently.
    private val programLimit = 1 shl 20
    private val depthLimit = 256

    private suspend fun showsALongTextCutAtTheSizeLimit(textBytes: Int) {
        val text = "a".repeat(textBytes)
        val program = "root = TextContent(\"$text\")"
        assertTrue("the program is past the core's size limit", program.encodeToByteArray().size > programLimit)
        val live = LiveProgram(createLibrary(listOf(textContent)))
        try {
            live.show(program)
            assertTrue("the core reported the cut", live.sizeLimited)
            val shown = (live.root?.prop("text") as? PropValue.Scalar)?.json?.content.orEmpty()
            assertTrue("what is shown really is cut", shown.length < text.length)

            live.show("root = TextContent(\"whole\")")
            assertFalse("the next whole program clears the cut", live.sizeLimited)
            assertFalse(live.depthLimited)
            assertEquals(PropValue.Scalar(JsonPrimitive("whole")), live.root?.prop("text"))
        } finally {
            live.dispose()
        }
    }

    @Test
    fun aTextJustPastTheProgramLimitIsReportedCut() = runBlocking { showsALongTextCutAtTheSizeLimit(programLimit) }

    @Test
    fun aTextEightTimesTheProgramLimitIsReportedCut() = runBlocking { showsALongTextCutAtTheSizeLimit(8 * programLimit) }

    @Test
    fun nestingPastTheDepthLimitIsReportedAndClearedByTheNextProgram() = runBlocking {
        val nesting = 5000
        assertTrue("the nesting is past the core's depth limit", nesting > depthLimit)
        val live = LiveProgram(createLibrary(listOf(textContent)))
        try {
            live.show("root = TextContent(" + "(".repeat(nesting))
            assertTrue("the core reported the cut", live.depthLimited)

            live.show("root = TextContent(\"whole\")")
            assertFalse(live.depthLimited)
            assertFalse(live.sizeLimited)
            assertEquals(PropValue.Scalar(JsonPrimitive("whole")), live.root?.prop("text"))
        } finally {
            live.dispose()
        }
    }
}
