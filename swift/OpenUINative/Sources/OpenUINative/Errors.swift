import Foundation
import OpenUIFFI

/// Why an OpenUINative call refused or failed: a `kind` to switch on and a
/// `message` to show. Every public throwing call in this package throws only
/// this (`throws(OpenUIError)`), so a caller never has to match on text. The
/// binding's refusals keep the binding's kind, one to one; the WebAssembly
/// package reports the same set of kinds as `error.kind`, under its own
/// spelling (`"InvalidJson"` there, `.invalidJSON` here; ADR 0007).
public struct OpenUIError: Error, Sendable, Equatable, CustomStringConvertible, LocalizedError {
    /// What went wrong. New cases may be added as the binding or this
    /// package gains refusals, so a `switch` over it keeps a `default`.
    public enum Kind: Sendable, Hashable, CaseIterable {
        // The binding's refusals, one kind per kind the core reports.

        /// The runtime session was disposed.
        case disposed
        /// A capability belongs to a different session.
        case foreignHandle
        /// An evaluation released the capability's generation.
        case staleHandle
        /// A mutation's continuation was already resumed.
        case consumedHandle
        /// A newer mutation suspension evicted this pending continuation.
        case evictedHandle
        /// No action exists at that place in the evaluated tree.
        case invalidHandle
        /// The value isn't a binding marker.
        case invalidMarker
        /// JSON given to the core is malformed, too deep, or the wrong shape.
        case invalidJSON
        /// The store refused a write before changing the value.
        case refusedWrite
        /// Evaluating an expression failed before a write or during an action.
        case evaluation
        /// A limit was exceeded: a response, a result or a store too large,
        /// or a counter used up.
        case limitExceeded
        /// The session's worker failed, or a function callback reentered it.
        case worker
        /// The component library's JSON Schema isn't valid JSON.
        case invalidSchema
        /// The prompt spec isn't valid JSON.
        case invalidPromptSpec
        /// The core can't make a system prompt from the spec; the message is
        /// the one upstream throws.
        case prompt
        /// Supplied reference values and a tool provider are mutually exclusive.
        case referenceConflict
        /// The tool call expired, belongs to another manager, or was completed.
        case invalidToolCall
        /// An action's mutation step was dropped because queries are paused.
        case queriesPaused
        /// A message attribute can't be URI-encoded, where upstream throws a
        /// `URIError`.
        case invalidAttribute

        // OpenUINative's own refusals.

        /// What was drawn has changed since, so nothing was done: an element
        /// or marker from an earlier program or evaluation.
        case stale
        /// A response is streaming, so nothing was done.
        case streaming
        /// The response on screen is a response bundle that blocks
        /// interaction; the message is the bundle's error.
        case blocked
        /// The element's props don't have the shape the call needs, such as
        /// a form control without a name.
        case unusableElement
        /// A tool provider is already attached.
        case toolProviderAttached
        /// An argument is outside what the call accepts.
        case invalidArgument
        /// What the core returned couldn't be read in Swift.
        case decode
        /// A newer call superseded this one, or its task was cancelled.
        case cancelled
        /// Anything else: a failure this package doesn't classify.
        case unexpected
    }

    /// The default host input byte limit was exceeded.
    public static let hostInputByteLimitExceeded = Self(.limitExceeded, "host input byte limit")

    public let kind: Kind
    public let message: String

    public init(_ kind: Kind, _ message: String) {
        self.kind = kind
        self.message = message
    }

    public var description: String { message }
    public var errorDescription: String? { message }

    /// Any error as one of these: itself, the binding's refusal by its kind,
    /// a cancellation, or `unexpected`.
    init(_ error: any Error) {
        switch error {
        case let error as OpenUIError: self = error
        case let error as OpenUiError: self.init(binding: error)
        case is CancellationError: self.init(.cancelled, "cancelled")
        case let error as JSONError: self.init(.decode, error.description)
        default: self.init(.unexpected, String(describing: error))
        }
    }

    /// The binding's refusal, with the binding's own wording. The switch is
    /// exhaustive on purpose: a kind added to the binding doesn't compile
    /// here until it has a kind of its own.
    init(binding error: OpenUiError) {
        switch error {
        case .Disposed: self.init(.disposed, "session disposed")
        case .ForeignHandle: self.init(.foreignHandle, "foreign capability")
        case .StaleHandle: self.init(.staleHandle, "stale action capability")
        case .ConsumedHandle: self.init(.consumedHandle, "continuation consumed")
        case .EvictedHandle: self.init(.evictedHandle, "continuation evicted")
        case .InvalidHandle: self.init(.invalidHandle, "invalid action path")
        case .InvalidMarker: self.init(.invalidMarker, "invalid binding marker")
        case let .InvalidJson(reason): self.init(.invalidJSON, reason)
        case let .RefusedWrite(reason): self.init(.refusedWrite, reason)
        case let .Evaluation(reason): self.init(.evaluation, reason)
        case let .LimitExceeded(reason): self.init(.limitExceeded, reason)
        case let .Worker(reason): self.init(.worker, reason)
        case let .InvalidSchema(reason): self.init(.invalidSchema, "invalid JSON Schema: \(reason)")
        case let .InvalidPromptSpec(reason): self.init(.invalidPromptSpec, "invalid prompt spec: \(reason)")
        case let .Prompt(reason): self.init(.prompt, reason)
        case .ReferenceConflict: self.init(.referenceConflict, "tool provider conflicts with supplied reference values")
        case .InvalidToolCall: self.init(.invalidToolCall, "invalid tool call identity")
        case let .QueriesPaused(reason): self.init(.queriesPaused, reason)
        case let .InvalidAttribute(reason): self.init(.invalidAttribute, reason)
        }
    }
}

/// Runs `body`, turning whatever it throws into an `OpenUIError`: the one
/// place a public call's untyped failures become typed.
@discardableResult
func bridged<T>(_ body: () throws -> T) throws(OpenUIError) -> T {
    do { return try body() } catch { throw OpenUIError(error) }
}
