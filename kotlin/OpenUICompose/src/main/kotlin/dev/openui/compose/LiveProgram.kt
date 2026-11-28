package dev.openui.compose

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.openui.compose.ffi.RuntimeSession
import dev.openui.compose.ffi.parse
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject

/**
 * A program kept live by a runtime session: its parse result evaluated by
 * the core against a store of state. Draw it with `ElementView(live)`;
 * [root] is Compose state, so a drawing recomposes when it changes.
 *
 * This first slice shows a whole program and nothing more; reading raw and
 * streaming responses, state, actions and tool calls come with #260 and the
 * tickets after it. One show runs at a time; the core's work and the
 * decoding of its tree run off the calling thread, and [root] and the limit
 * flags change together on the calling thread once they are done.
 */
public class LiveProgram(internal val library: Library) {
    private val session: RuntimeSession = core { RuntimeSession(library.inner, null) }
    private val shows = Mutex()

    /** What one show published, so no reader sees a new root with old flags. */
    private class Display(val root: Element?, val sizeLimited: Boolean, val depthLimited: Boolean)

    private var display: Display by mutableStateOf(Display(null, sizeLimited = false, depthLimited = false))

    /** The evaluated tree's root; null before a program is shown, or when the program has no root. */
    public val root: Element? get() = display.root

    /**
     * The core cut what is shown at a size limit: the program, its parse
     * result or the evaluated tree grew past one. What is shown is not the
     * whole program; `ElementView(live)` says so above it.
     */
    public val sizeLimited: Boolean get() = display.sizeLimited

    /** The core cut what is shown at its nesting depth limit, while parsing or evaluating. */
    public val depthLimited: Boolean get() = display.depthLimited

    /**
     * Shows a whole program: the core parses it against the library, the
     * session takes it in place of the program it held, and the tree is
     * evaluated. A refused program leaves what was shown as it was.
     */
    public suspend fun show(program: String) {
        shows.withLock {
            display = withContext(Dispatchers.Default) { evaluate(program) }
        }
    }

    private fun evaluate(program: String): Display {
        var sizeLimited = false
        var depthLimited = false
        val json = core {
            val parsed = parse(program, library.inner, library.root)
            parsed.program.use { parsedProgram ->
                val evaluation = session.replaceProgram(parsedProgram, null, null, true).evaluation
                evaluation.evaluation.close()
                sizeLimited = parsed.sizeLimited || evaluation.sizeLimited
                depthLimited = parsed.depthLimited || evaluation.depthLimited
                evaluation.json
            }
        }
        val tree = try {
            Json.parseToJsonElement(json) as? JsonObject
        } catch (error: SerializationException) {
            throw OpenUIException(OpenUIException.Kind.Decode, "the evaluated tree isn't JSON: ${error.message}", error)
        } ?: throw OpenUIException(OpenUIException.Kind.Decode, "the evaluated tree isn't a JSON object")
        val rootJson = tree["root"] ?: JsonNull
        val root = if (rootJson == JsonNull) null else elementOrNull(rootJson)
            ?: throw OpenUIException(OpenUIException.Kind.Decode, "the evaluated tree's root isn't an element")
        return Display(root, sizeLimited, depthLimited)
    }

    /** Ends the runtime session. Later calls refuse with [OpenUIException.Kind.Disposed]. */
    public fun dispose() {
        session.dispose()
    }
}
