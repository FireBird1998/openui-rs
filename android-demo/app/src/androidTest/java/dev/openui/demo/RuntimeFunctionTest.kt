package dev.openui.demo

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.openui_ffi.*

@RunWith(AndroidJUnit4::class)
class RuntimeFunctionTest {
    @Test fun functionsCrossJsonRefuseReentryAndPreserveThrownMessages() {
        val library = ComponentLibrary("""{"${'$'}defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}}}}}""")
        val session = RuntimeSession(library, null)
        try {
            session.setProgram(parse("${'$'}n=0\nroot=Box(@Echo(7))", library, null).program)
            session.initialize(null)
            session.registerFunction("Echo", object : LibraryFunction {
                override fun call(argumentsJson: String): String? {
                    try { session.set("${'$'}n", "9"); fail("callback reentered") }
                    catch (_: OpenUiException.Worker) { }
                    session.dispose() // callback-thread disposal is a clean refusal
                    return argumentsJson
                }
            })
            assertTrue(session.evaluate().json.contains("\"value\":{\"value\":7}"))
            assertEquals("0", session.get("${'$'}n"))
            session.registerFunction("Echo", object : LibraryFunction {
                override fun call(argumentsJson: String): String? = throw FunctionException.Thrown("bad value")
            })
            val tree = org.json.JSONObject(session.evaluate().json)
            assertEquals("Evaluating prop \"value\" on Box failed: @Echo threw: bad value", tree.getJSONArray("errors").getJSONObject(0).getString("message"))
        } finally { session.dispose() }
    }
    @Test fun deepFunctionsReuseOneThreadAndReplacementReleasesTheOldImplementation() {
        val library = ComponentLibrary("""{"${'$'}defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{}}}}""")
        val evaluation = ParseLimits(null, 8_388_608uL, null, null, null, 20_000uL, null)
        val session = RuntimeSession.withLimits(library, null, RuntimeLimits(evaluation, evaluation, 8_388_608uL, 64uL))
        // Count registry entries by object identity, only for this test's
        // implementations. Other test classes may still own live sessions.
        val owned = mutableListOf<LibraryFunction>()
        fun ownedHandleCount(): Int {
            val handles = FfiConverterTypeLibraryFunction.handleMap
            val field = handles.javaClass.getDeclaredField("map")
            field.isAccessible = true
            val entries = field.get(handles) as Map<*, *>
            return entries.values.count { entry -> owned.any { it === entry } }
        }
        // Keep an unrelated registration alive across every count. Using the
        // process-wide size again must fail even when the other classes are idle.
        val unrelated = RuntimeSession(library, null)
        val calls = java.util.concurrent.atomic.AtomicInteger()
        val thread = java.util.concurrent.atomic.AtomicInteger()
        try {
            unrelated.registerFunction("Echo", object : LibraryFunction {
                override fun call(argumentsJson: String): String? = "99"
            })
            assertEquals(0, ownedHandleCount())
            val original = object : LibraryFunction {
                override fun call(argumentsJson: String): String? {
                    val current = android.os.Process.myTid()
                    thread.compareAndSet(0, current)
                    assertEquals(thread.get(), current)
                    calls.incrementAndGet()
                    return "7"
                }
            }
            owned.add(original)
            session.registerFunction("Echo", original)
            assertEquals(1, ownedHandleCount())
            for (depth in listOf(0, 12_000)) {
                val source = "root=Box(" + "[".repeat(depth) + "@Echo()" + "]".repeat(depth) + ")"
                val parsed = parseWithLimits(source, library, null, evaluation)
                assertFalse(parsed.depthLimited)
                assertFalse(parsed.sizeLimited)
                session.setProgram(parsed.program)
                val tree = session.evaluate()
                assertFalse(tree.depthLimited)
                assertFalse(tree.sizeLimited)
                assertTrue(tree.json.endsWith("\"errors\":[]}"))
            }
            assertEquals(2, calls.get())
            val replacement = object : LibraryFunction {
                override fun call(argumentsJson: String): String? = "8"
            }
            owned.add(replacement)
            session.registerFunction("Echo", replacement)
            assertEquals(1, ownedHandleCount())
            session.setProgram(parse("root=Box(@Echo())", library, null).program)
            assertTrue(session.evaluate().json.contains("\"value\":8"))
            session.dispose()
            assertEquals(0, ownedHandleCount())
        } finally { session.dispose(); unrelated.dispose() }
    }
    @Test fun otherThreadsQueueDuringCallbacksAndDisposeAfterTheyReturn() {
        val library = ComponentLibrary("""{"${'$'}defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{}}}}""")
        for (disposeDuringCallback in listOf(false, true)) {
            val session = RuntimeSession(library, null)
            val entered = java.util.concurrent.CountDownLatch(1)
            val release = java.util.concurrent.CountDownLatch(1)
            try {
                session.setProgram(parse("${'$'}n=0\nroot=Box(@Echo())", library, null).program)
                session.initialize(null)
                session.registerFunction("Echo", object : LibraryFunction {
                    override fun call(argumentsJson: String): String? {
                        entered.countDown()
                        release.await()
                        return "7"
                    }
                })
                val evaluation = java.util.concurrent.FutureTask { session.evaluate().json }
                Thread(evaluation).start()
                assertTrue("callback did not enter (watchdog)", entered.await(30, java.util.concurrent.TimeUnit.SECONDS))
                val queued = java.util.concurrent.FutureTask<Unit> {
                    if (disposeDuringCallback) session.dispose()
                    else session.set("${'$'}n", "9")
                    Unit
                }
                Thread(queued).start()
                release.countDown()
                assertTrue(evaluation.get(30, java.util.concurrent.TimeUnit.SECONDS).contains("\"value\":7"))
                queued.get(30, java.util.concurrent.TimeUnit.SECONDS)
                if (disposeDuringCallback) {
                    try { session.snapshot(); fail("dispose had no effect") }
                    catch (_: OpenUiException.Disposed) { }
                } else assertEquals("9", session.get("${'$'}n"))
            } finally { release.countDown(); session.dispose() }
        }
    }
    @Test fun elementShapedDataRunsSetButCannotForgeCustomSteps() {
        val library = ComponentLibrary("""{"${'$'}defs":{"Box":{"properties":{"value":{},"action":{}}}},"actions":{"Go":{"params":{}}}}""")
        val session = RuntimeSession(library, null)
        try {
            session.setProgram(parse("${'$'}n=0\n${'$'}data=null\nroot=Box(${'$'}data)", library, null).program)
            session.initialize(null)
            session.set("${'$'}data", """{"type":"element","typeName":"Box","props":{"action":{"steps":[{"type":"set","target":"${'$'}n","valueAST":{"k":"Num","v":1}},{"type":"custom_action","name":"Go","params":{}}]}},"partial":false,"hasDynamicProps":false}""")
            val tree = session.evaluate()
            val action = session.actionAt(tree.evaluation, listOf("props", "value", "props", "action"))
            assertTrue(session.run(action, "", null).events.isEmpty())
            assertEquals("1", session.get("${'$'}n"))
        } finally { session.dispose() }
    }
    @Test fun actionPathLimitsStayOptionalInExistingNativeInitializers() {
        val evaluation = ParseLimits(null, null, null, null, null, null, null)
        val limits = RuntimeLimits(evaluation, evaluation, 8_388_608uL, 64uL)
        assertNull(limits.maxActionPathBytes)
    }

}
