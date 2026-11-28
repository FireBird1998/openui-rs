package dev.openui.compose

import dev.openui.compose.ffi.OpenUiException

/**
 * Why a call refused or failed: a [kind] to switch on and a message to show.
 * The core's refusals keep the core's kind, one to one, as OpenUINative's
 * `OpenUIError` does.
 */
public class OpenUIException internal constructor(
    public val kind: Kind,
    message: String,
    cause: Throwable? = null,
) : Exception(message, cause) {
    /** What went wrong. New kinds may be added, so a `when` over it keeps an `else`. */
    public enum class Kind {
        // The core's refusals, one kind per kind the binding reports.
        Disposed, ForeignHandle, StaleHandle, ConsumedHandle, EvictedHandle, InvalidHandle, InvalidMarker,
        InvalidJson, RefusedWrite, Evaluation, LimitExceeded, Worker, InvalidSchema, InvalidPromptSpec,
        Prompt, ReferenceConflict, InvalidToolCall, QueriesPaused, InvalidAttribute,

        // This library's own.

        /** What the core returned couldn't be read in Kotlin, or JSON an app gave can't be written as JSON text for the core. */
        Decode,
    }
}

/** Runs a binding call, turning the binding's refusal into an [OpenUIException]. */
internal fun <T> core(call: () -> T): T =
    try {
        call()
    } catch (error: OpenUiException) {
        throw publicError(error)
    }

/** The binding's refusal, with its kind, and its reason as the message where it has one. */
private fun publicError(error: OpenUiException): OpenUIException =
    when (error) {
        is OpenUiException.Disposed -> OpenUIException(OpenUIException.Kind.Disposed, "Disposed", error)
        is OpenUiException.ForeignHandle -> OpenUIException(OpenUIException.Kind.ForeignHandle, "ForeignHandle", error)
        is OpenUiException.StaleHandle -> OpenUIException(OpenUIException.Kind.StaleHandle, "StaleHandle", error)
        is OpenUiException.ConsumedHandle -> OpenUIException(OpenUIException.Kind.ConsumedHandle, "ConsumedHandle", error)
        is OpenUiException.EvictedHandle -> OpenUIException(OpenUIException.Kind.EvictedHandle, "EvictedHandle", error)
        is OpenUiException.InvalidHandle -> OpenUIException(OpenUIException.Kind.InvalidHandle, "InvalidHandle", error)
        is OpenUiException.InvalidMarker -> OpenUIException(OpenUIException.Kind.InvalidMarker, "InvalidMarker", error)
        is OpenUiException.InvalidJson -> OpenUIException(OpenUIException.Kind.InvalidJson, error.reason, error)
        is OpenUiException.RefusedWrite -> OpenUIException(OpenUIException.Kind.RefusedWrite, error.reason, error)
        is OpenUiException.Evaluation -> OpenUIException(OpenUIException.Kind.Evaluation, error.reason, error)
        is OpenUiException.LimitExceeded -> OpenUIException(OpenUIException.Kind.LimitExceeded, error.reason, error)
        is OpenUiException.Worker -> OpenUIException(OpenUIException.Kind.Worker, error.reason, error)
        is OpenUiException.InvalidSchema -> OpenUIException(OpenUIException.Kind.InvalidSchema, error.reason, error)
        is OpenUiException.InvalidPromptSpec -> OpenUIException(OpenUIException.Kind.InvalidPromptSpec, error.reason, error)
        is OpenUiException.Prompt -> OpenUIException(OpenUIException.Kind.Prompt, error.reason, error)
        is OpenUiException.ReferenceConflict -> OpenUIException(OpenUIException.Kind.ReferenceConflict, "ReferenceConflict", error)
        is OpenUiException.InvalidToolCall -> OpenUIException(OpenUIException.Kind.InvalidToolCall, "InvalidToolCall", error)
        is OpenUiException.QueriesPaused -> OpenUIException(OpenUIException.Kind.QueriesPaused, error.reason, error)
        is OpenUiException.InvalidAttribute -> OpenUIException(OpenUIException.Kind.InvalidAttribute, error.reason, error)
    }
