package dev.openui.demo

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.charset.MalformedInputException
import uniffi.openui_ffi.*

/** Runs on Android because the binding uses Android's native cleaner and JNA. */
@RunWith(AndroidJUnit4::class)
class RuntimeSessionTest {
    @Test fun bindingsActionsAndReleasedGenerations() {
        val library = ComponentLibrary("""{"${'$'}defs":{"Box":{"properties":{"value":{},"action":{}}}}}""")
        val session = RuntimeSession(library, null)
        try {
            session.markBinding("Box", "value")
            val output = parse("${'$'}n=1\nroot=Box(${'$'}n,@Set(${'$'}n,${'$'}n+1))", library, null)
            session.setProgram(output.program)
            assertTrue(session.initialize(null).changed)
            val tree = session.evaluate()
            val action = session.actionAt(tree.evaluation, listOf("props", "action"))
            val marker = """{"__reactive":"assign","target":"${'$'}n","expr":{"k":"StateRef","n":"${'$'}value"}}"""
            assertTrue(session.writeBinding(marker, "7"))
            assertTrue(session.run(action, "", null).events.isEmpty())
            assertEquals("8", session.get("${'$'}n"))
            assertEquals("""{"${'$'}n":8}""", session.snapshot())
            session.evaluate()
            try {
                session.run(action, "", null)
                fail("A released generation ran")
            } catch (_: OpenUiException.StaleHandle) { }

            // This exception comes from generated Kotlin lowering, before Rust.
            try {
                session.set("${'$'}text", "\"" + String(charArrayOf('\uD800')) + "\"")
                fail("Expected java.nio.charset.MalformedInputException")
            } catch (_: MalformedInputException) { }
            assertNull(session.get("${'$'}text"))
            session.set("${'$'}text", "\"\\ud800\"")
            assertEquals("\"\\ud800\"", session.get("${'$'}text"))
            session.dispose()
            try { session.version(); fail("A disposed session accepted a call") }
            catch (_: OpenUiException.Disposed) { }
        } finally { session.dispose() }
    }

    @Test fun splitSurrogatePairsAreBufferedAndLoneUnitsAreReportedByName() {
        val library = ComponentLibrary("""{"${'$'}defs":{"Box":{"properties":{"value":{}}}}}""")
        val direct = StreamSession(library, null)
        val before = direct.result().json
        try {
            direct.push("root=Box(\"\uD83D")
            fail("Expected java.nio.charset.MalformedInputException from StreamSession.push")
        } catch (_: MalformedInputException) { }
        assertEquals(before, direct.result().json)

        val stream = StreamSession(library, null)
        val buffered = Utf16StreamBuffer(stream)
        buffered.push("root=Box(\"\uD83D")
        assertNotNull(buffered.push(""))
        buffered.push("\uDE00\")")
        val expected = parse("root=Box(\"\uD83D\uDE00\")", library, null)
        assertEquals(expected.json, buffered.finish().json)

        val unfinished = Utf16StreamBuffer(StreamSession(library, null))
        unfinished.push("root=Box(\"\uD83D")
        try {
            unfinished.finish()
            fail("Expected java.nio.charset.MalformedInputException at end of stream")
        } catch (_: MalformedInputException) { }
    }
    @Test fun loneLowSurrogateRefusesWithoutChangingBufferAndPendingHighCanBeDiscarded() {
        val library = ComponentLibrary("""{"${'$'}defs":{"Box":{"properties":{"value":{}}}}}""")
        val stream = StreamSession(library, null)
        val buffered = Utf16StreamBuffer(stream)
        val before = stream.result().json
        try { buffered.push("\uDE00"); fail("lone low surrogate accepted") }
        catch (_: MalformedInputException) { }
        assertEquals(before, stream.result().json)
        assertNotNull(buffered.push("root=Box(\"\uD83D"))
        try { buffered.finish(); fail("pending high surrogate accepted") }
        catch (_: MalformedInputException) { }
        buffered.discardPending()
        buffered.push("ok\")")
        assertEquals(parse("root=Box(\"ok\")", library, null).json, buffered.finish().json)
    }

}
