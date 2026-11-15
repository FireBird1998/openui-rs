package dev.openui.demo

import java.nio.charset.MalformedInputException
import uniffi.openui_ffi.ParseOutput
import uniffi.openui_ffi.StreamSession

/** Caller-side UTF-16 chunk buffering. Use on one stream-driving thread.
 * UniFFI reports malformed UTF-16 before Rust receives a string. Only a trailing
 * high surrogate can be a valid split pair; other malformed input still throws.
 */
class Utf16StreamBuffer(private val stream: StreamSession) {
    private var trailing = ""

    fun push(chunk: String): ParseOutput? {
        val joined = trailing + chunk
        val held = if (joined.lastOrNull()?.isHighSurrogate() == true) 1 else 0
        val ready = joined.dropLast(held)
        val output = if (ready.isEmpty()) null else stream.push(ready)
        // Update only after successful lowering, leaving the buffer unchanged
        // if an internal lone surrogate is rejected by the generated encoder.
        trailing = joined.takeLast(held)
        return output
    }

    fun finish(): ParseOutput {
        if (trailing.isNotEmpty()) throw MalformedInputException(1)
        return stream.finish()
    }
}
