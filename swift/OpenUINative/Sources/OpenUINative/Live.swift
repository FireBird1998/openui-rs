import Foundation
import Observation
import OpenUIFFI

/// A program kept live by a runtime session: its parse result evaluated
/// against a store of state, redrawn whenever the store changes. Draw it with
/// `ElementView(live:)`; built-in buttons run their actions, native form
/// controls read and write through binding markers or form fields, and
/// every host event goes to `onHostEvent`.
///
/// Every call reaches the session's own worker thread and waits for it (ADR
/// 0006), so a call costs a round trip plus the work. Writes and clicks
/// re-evaluate the tree before they return, on the main actor, so the drawing
/// and a text field being typed in never disagree. That's cheap enough for
/// the programs a model writes: a release build redraws `bench/dashboard`
/// (3.5 KB) in about 0.2 ms and eight copies of it (30 KB) in about 1.5 ms,
/// roughly 22 µs per KB of the evaluated tree's JSON, most of it decoding in
/// Swift. Past about 350 KB of tree, a keystroke costs more than half a frame
/// (TESTING.md, "The runtime session's cost").
@MainActor @Observable
public final class LiveProgram {
    /// The evaluated tree's root; nil before a program is shown, or when the
    /// program has no root.
    public var root: Element? { display.root }
    /// What failed while evaluating the tree, in the runtime's order.
    public var errors: [RuntimeIssue] { display.errors }
    /// The store's snapshot: every state variable and form field that has a
    /// value, by name, in the store's order.
    public var state: JSONValue { display.state }
    /// The displayed session's store version. Discarding a preview restores
    /// the complete session's version along with its state.
    public var version: UInt64 { display.version }
    /// The last evaluation was cut at the runtime's size or work limit.
    public var sizeLimited: Bool { display.sizeLimited }
    /// The last evaluation was cut at the runtime's depth limit.
    public var depthLimited: Bool { display.depthLimited }
    /// Why a built-in interaction stopped, or why a disposable preview could
    /// not project an admitted app write. A projection warning does not undo
    /// the complete write. Built-in attempts clear the preceding failure.
    public private(set) var failure: String?
    /// How long the last evaluation took, decoding included.
    public var evaluation: Duration? { display.evaluation }
    /// A response is still streaming into the program. The app sets it while
    /// the program's text is arriving and clears it once the response is
    /// complete. Meanwhile built-in buttons, inputs and selects are disabled,
    /// a click that still arrives is ignored, and `run` refuses, as upstream's
    /// renderer does until its response is complete. A stream push remains
    /// gated until a finish result is shown, even if this flag is cleared.
    /// `write` refuses too; app state writes with `set` and `resume` still work.
    public var isStreaming: Bool {
        get { requestedStreaming || stepIsStreaming || !interactionSuspensions.isEmpty }
        set { requestedStreaming = newValue }
    }
    /// Built-in controls can interact only with a complete, usable runtime.
    /// A failed complete replacement is refused until a later complete show
    /// succeeds; the binding cannot roll that replacement back yet.
    public var acceptsInteractions: Bool { !isStreaming && !completeInvalid }
    private var completeInvalid = false
    /// The app-owned request, separate from an immutable preview step's phase.
    /// Temporary hosts use suspendInteractions(), so they never overwrite
    /// an app request made while their asynchronous work is suspended.
    public var isStreamingRequested: Bool { requestedStreaming }
    private var requestedStreaming = false
    private var stepIsStreaming: Bool { display.streaming }
    private var interactionSuspensions: Set<UUID> = []

    /// Temporarily gates clicks without writing the app-owned streaming
    /// request. The owner must release its suspension when its work ends.
    public func suspendInteractions() -> InteractionSuspension {
        let id = UUID()
        interactionSuspensions.insert(id)
        return InteractionSuspension(self, id: id)
    }

    @MainActor public final class InteractionSuspension {
        private weak var program: LiveProgram?
        private let id: UUID
        fileprivate init(_ program: LiveProgram, id: UUID) { self.program = program; self.id = id }
        /// Releases only this owner's gate. Repeated releases have no effect.
        public func release() {
            program?.interactionSuspensions.remove(id)
            program = nil
        }
    }

    /// Receives every host event, in order: what the program asks the app to
    /// do (send a message to the assistant, open a URL, a custom action, a
    /// query or mutation to run). It's called after the store has changed and
    /// the tree has been evaluated again.
    @ObservationIgnored public var onHostEvent: @MainActor (HostEvent) -> Void

    /// Internal so tests can reach it: the viewer's count its pending
    /// mutations, and the package's dispose it under a redraw.
    @ObservationIgnored let session: RuntimeSession
    @ObservationIgnored private let library: Library
    @ObservationIgnored private let promptSpec: String?
    @ObservationIgnored private let bindings: [(component: String, prop: String)]
    @ObservationIgnored private var previewSession: RuntimeSession?
    @ObservationIgnored private var completeDisplay: Display?
    @ObservationIgnored private var completeStep: Step?
    /// The pristine state variables at a store version, read once.
    @ObservationIgnored private var pristineCache: (version: UInt64, names: Set<String>)?
    private var handle: EvaluationHandle? { display.handle }
    @ObservationIgnored private var hasProgram = false
    /// The last evaluated tree's root as the runtime gave it.
    var rootJSON: JSONValue? { display.rootJSON }
    /// This live program's identity, carried by its tree's elements and markers.
    private var id: UUID { display.id }
    /// Which evaluation `root` is; it rises with each one.
    private var generation: UInt64 { display.generation }

    /// One observable publication: no observer sees a new root with an old
    /// store snapshot, handle, identity, limit flags or interaction gate.
    private struct Display {
        var root: Element?
        var errors: [RuntimeIssue] = []
        var state: JSONValue = .object([])
        var version: UInt64 = 0
        var sizeLimited = false
        var depthLimited = false
        var evaluation: Duration?
        var rootJSON: JSONValue?
        var handle: EvaluationHandle?
        var id = UUID()
        var generation: UInt64 = 0
        var streaming = false
    }
    private var display = Display()

    /// A session for programs in `library`. The props that are bindings come
    /// from `promptSpec` (its signatures' `$binding<…>` props; the library
    /// object or the whole spec) and from `bindings`, for components the spec
    /// doesn't describe.
    public init(
        library: Library,
        promptSpec: String? = nil,
        bindings: [(component: String, prop: String)] = [],
        onHostEvent: @escaping @MainActor (HostEvent) -> Void = { _ in }
    ) throws {
        self.library = library
        self.promptSpec = promptSpec
        self.bindings = bindings
        session = try RuntimeSession(library: library.inner, promptSpec: promptSpec)
        for binding in bindings {
            try session.markBinding(component: binding.component, prop: binding.prop)
        }
        self.onHostEvent = onHostEvent
    }

    /// Shows a parse result: the store takes its state declarations (a value
    /// set before survives; a declared starting value fills in or replaces
    /// one still pristine), `persisted` values win over both, and the tree is
    /// evaluated. Every accepted complete show revokes older elements and
    /// markers, even for identical programs. A stream push uses a separate,
    /// noninteractive preview session; `discardPreview()` restores the complete
    /// session after a parser failure. Call with each step of a stream. Invalid
    /// persisted input is checked before the program is replaced. The binding
    /// has no transaction across setProgram/initialize/evaluate: an error after
    /// replacement can still invalidate the preceding runtime handles.
    public func show(_ step: Step, persisted: JSONValue? = nil) throws {
        // Admission happens before any phase, display or runtime mutation,
        // including when replacing an existing preview.
        try preflight(persisted)
        let oldDisplay = display
        let oldCompleteDisplay = completeDisplay
        let oldCompleteStep = completeStep
        let oldPreview = previewSession
        let oldCache = pristineCache
        let oldFailure = failure
        do { try apply(step, persisted: persisted) }
        catch {
            if completeInvalid {
                // Known Rust transaction follow-up: an accepted complete
                // replacement cannot restore the underlying store/handles.
                discardPreview()
            } else {
                if previewSession !== oldPreview { previewSession?.dispose() }
                previewSession = oldPreview
                completeDisplay = oldCompleteDisplay
                completeStep = oldCompleteStep
                display = oldDisplay
                pristineCache = oldCache
                failure = oldFailure
            }
            throw error
        }
    }

    private func preflight(_ persisted: JSONValue?) throws {
        // Check every caller-supplied persisted value before replacing the
        // runtime program. initialize's JSON admission can throw after setProgram.
        // Use the binding's own reader/limits, not a second Swift approximation.
        if let persisted {
            let preflight = try RuntimeSession(library: library.inner, promptSpec: nil)
            defer { preflight.dispose() }
            _ = try preflight.initialize(persisted: persisted.compact)
        }
    }

    private func apply(_ step: Step, persisted: JSONValue?) throws {
        if step.isStreaming {
            try showPreview(step, persisted: persisted)
            return
        }
        guard generation < UInt64.max else { throw LiveError(description: "evaluation generation exhausted") }
        let wasInvalid = completeInvalid
        completeInvalid = true
        do { try session.setProgram(program: step.parsed) }
        catch { completeInvalid = wasInvalid; throw error }
        pristineCache = nil
        let initialized = try session.initialize(persisted: persisted?.compact)
        let next = try evaluated(session, from: completeDisplay ?? display, programID: UUID(), streaming: false)
        previewSession?.dispose()
        previewSession = nil
        completeDisplay = nil
        completeStep = step
        display = next
        completeInvalid = false
        hasProgram = true
        failure = initialized.refused.isEmpty ? nil
            : initialized.refused.map { "\($0.name): \(Self.describe($0.error))" }.joined(separator: "\n")
    }

    /// Discards streamed previews after a parser failure or cancellation.
    /// The complete session was never replaced: its store, pristine values,
    /// pending mutations, elements and custom-action authority remain intact.
    /// Retained preview controls remain stale after the complete drawing returns.
    /// The app-owned isStreaming request is preserved; its owner clears it.
    public func discardPreview() {
        previewSession?.dispose()
        previewSession = nil
        pristineCache = nil
        if let completeDisplay { display = completeDisplay }
        completeDisplay = nil
    }

    private func showPreview(_ step: Step, persisted: JSONValue?) throws {
        guard !completeInvalid else { throw Self.stale }
        do {
            if previewSession == nil {
                let preview = try RuntimeSession(library: library.inner, promptSpec: promptSpec)
                for binding in bindings { try preview.markBinding(component: binding.component, prop: binding.prop) }
                // Keep the actual complete session. Only the noninteractive
                // drawing uses a JSON projection; copied plans gain no authority.
                completeDisplay = display
                previewSession = preview
                if let completeStep {
                    try preview.setProgram(program: completeStep.parsed)
                    _ = try preview.initialize(persisted: nil)
                }
            }
            guard let previewSession else { throw LiveError(description: "no preview session") }
            try previewSession.setProgram(program: step.parsed)
            let projectionWarning = try copyEditedState(to: previewSession, from: completeDisplay ?? display)
            let initialized = try previewSession.initialize(persisted: persisted?.compact)
            let next = try evaluated(previewSession, from: display, programID: UUID(), streaming: true)
            pristineCache = nil
            display = next
            let initializationWarning = initialized.refused.isEmpty ? nil : initialized.refused.map {
                "\($0.name): \(Self.describe($0.error))"
            }.joined(separator: "\n")
            let warnings = [projectionWarning, initializationWarning].compactMap { $0 }
            failure = warnings.isEmpty ? nil : warnings.joined(separator: "\n")
        }
    }

    private func copyEditedState(to preview: RuntimeSession, from complete: Display) throws -> String? {
        guard case let .object(entries) = complete.state else { return nil }
        let dirty = try entries.filter { try !session.isPristine(name: $0.key) }
        let initialized = try preview.initialize(persisted: JSONValue.object(dirty).compact)
        guard !initialized.refused.isEmpty else { return nil }
        return "preview projection: " + initialized.refused.map {
            "\($0.name): \(Self.describe($0.error))"
        }.joined(separator: "\n")
    }

    /// A preview is disposable. Its budget must not turn an already admitted
    /// canonical app write into a throwing, partially applied refusal.
    private func mirror(_ write: (RuntimeSession) throws -> Void) -> String? {
        guard let previewSession else { return nil }
        do { try write(previewSession); return nil }
        catch { return "preview projection: " + Self.describe(error) }
    }

    // MARK: State

    /// A state variable's value (name with its `$`); nil when it has none.
    public func value(of name: String) -> JSONValue? {
        state[name]
    }

    /// The state variables in `state` that still hold their declared starting
    /// value, which a later declaration may replace. The session answers for
    /// one name at a time, so this asks once per name after each change to the
    /// store and keeps the answer until the next.
    public var pristineNames: Set<String> {
        if let cached = pristineCache, cached.version == version { return cached.names }
        guard case let .object(entries) = state else { return [] }
        let currentSession = previewSession ?? session
        let names = Set(entries.map(\.key).filter { (try? currentSession.isPristine(name: $0)) ?? false })
        pristineCache = (version, names)
        return names
    }

    /// Sets a state variable (nil makes it undefined) and redraws if that
    /// changed anything. Throws if the complete store refuses the value.
    /// A preview projection refusal is reported in failure; the admitted
    /// complete write survives and is visible after discarding/finishing it.
    public func set(_ name: String, to value: JSONValue?) throws {
        _ = try session.set(name: name, json: value?.compact)
        let warning = mirror { _ = try $0.set(name: name, json: value?.compact) }
        let redrawWarning = try refresh()
        if let warning = warning ?? redrawWarning { failure = warning }
    }

    /// Writes a value through a binding marker from the tree: the marker's
    /// expression turns it into what's stored. A marker from this program's
    /// current evaluation writes. One from an earlier evaluation (a second
    /// keystroke that arrived before the redraw) writes only while the same
    /// marker is at its place in the tree now; otherwise it throws and
    /// changes nothing.
    public func write(_ marker: BindingMarker, _ value: JSONValue) throws {
        guard !isStreaming else { throw Self.streaming }
        guard !completeInvalid else { throw Self.stale }
        guard let origin = marker.origin, origin.program == id else { throw Self.stale }
        if origin.generation != generation {
            guard case let .binding(now)? = locate(origin.steps)?.value, now.json == marker.json else { throw Self.stale }
        }
        _ = try session.writeBinding(marker: marker.json.compact, json: value.compact)
        try refresh()
    }

    /// An unbound form field's value: in `form`'s fields, or a top-level
    /// field when `form` is nil; nil when it has none. Scalar and wrapped values
    /// come from `state`. An object without a `value` key needs the runtime's
    /// getter: JSON omits undefined properties, so the snapshot alone cannot
    /// distinguish an undefined field from lookalike object data.
    public func field(form: String?, name: String) -> JSONValue? {
        let raw: JSONValue?
        if let form, !form.isEmpty {
            guard case .object? = state[form] else { return nil }
            raw = state[form]?[name]
        } else {
            raw = state[name]
        }
        guard case let .object(entries)? = raw else { return raw }
        if let value = entries.first(where: { $0.key == "value" }) { return value.value }
        guard let json = try? (previewSession ?? session).getField(form: form, name: name) else { return nil }
        return try? JSONValue.parse(json)
    }

    /// Writes an unbound form field and redraws if that changed anything.
    /// `componentType` is kept beside the value and travels in events' form
    /// state; upstream's own Input and Select give none, while a component
    /// that writes its field itself (such as Chips) gives its name. Complete
    /// admission determines success; preview refusals are reported in failure.
    public func setField(form: String?, componentType: String? = nil, name: String, to value: JSONValue?) throws {
        _ = try session.setField(form: form, componentType: componentType, name: name, json: value?.compact, save: true)
        let warning = mirror {
            _ = try $0.setField(form: form, componentType: componentType, name: name, json: value?.compact, save: true)
        }
        let redrawWarning = try refresh()
        if let warning = warning ?? redrawWarning { failure = warning }
    }

    /// Writes what a person typed or picked in a built-in form control
    /// drawn from `element`: through its binding marker, or else into its
    /// form field. Refused, changing nothing, when what was drawn is gone
    /// (see `current(_:form:)`), or while streaming. Entering the value already
    /// held is a no-op, including when the binding transforms new values.
    /// Chips/OptionCards include their component type and accept nil when a
    /// single selection is cleared; a binding refuses undefined with LiveError.
    public func enter(_ value: JSONValue?, into element: Element, form: String? = nil) throws {
        guard !isStreaming else { throw Self.streaming }
        guard let element = current(element, form: form) else { throw Self.stale }
        let props = PropReader(element: element)
        let name = try props.text("name").plain
        let field = FieldValue(props, name: name)
        guard value != field.value(in: self, form: form, includingCompositeDefaults: true) else { return }
        if case let .binding(marker)? = props.value("value") {
            guard let value else { throw LiveError(description: "a binding cannot be set to undefined") }
            try write(marker, value)
        } else {
            let type = SelectionField.isSelection(element) ? element.typeName : nil
            try setField(form: form, componentType: type, name: name, to: value)
        }
    }

    /// Hydrates only a missing selection default, never while streaming or
    /// from an obsolete view. The runtime owns the undefined/default guard.
    func initializeSelection(_ element: Element, form: String?) throws {
        guard !isStreaming, let element = current(element, form: form) else { return }
        let props = PropReader(element: element)
        guard let value = SelectionField.defaultValue(props) else { return }
        _ = try session.initializeField(form: form, componentType: element.typeName,
            name: try props.text("name").plain, json: value.compact, streaming: false)
        try refresh()
    }

    // MARK: Actions

    /// Whether `element` is from this program's current evaluation: what's
    /// drawn from it is what the tree holds now.
    public func isCurrent(_ element: Element) -> Bool {
        guard !completeInvalid, let origin = element.origin else { return false }
        return origin.program == id && origin.generation == generation
    }

    /// What a click or a write from `element`, drawn inside `form`, acts on.
    /// While `element` is from the current evaluation, itself. From an earlier
    /// evaluation of this program (an event that arrived before the redraw),
    /// the element now at the same place, but only if it is exactly what was
    /// drawn (the same component, props, actions and markers) inside a form
    /// of the same name. Otherwise nil: what was drawn there is gone.
    func current(_ element: Element, form: String?) -> Element? {
        guard !completeInvalid, let origin = element.origin, origin.program == id else { return nil }
        if origin.generation == generation { return element }
        guard let found = locate(origin.steps), case let .element(now)? = found.value,
              found.form == form, now.json == element.json else { return nil }
        return now
    }

    /// Registers an app component's validation value reader. Its `name` and
    /// structured `rules` props use the same conventions as Input. The reader
    /// receives the current element, live store and enclosing form name; it
    /// must only read state. Register before drawing; registering again replaces
    /// the reader. Pass nil to unregister. Hidden tabs and nested forms are
    /// handled by the same traversal as built-in fields.
    public func registerValidationComponent(
        _ component: String,
        value: (@MainActor (Element, LiveProgram, String?) -> JSONValue?)?
    ) {
        validationComponents[component] = value
    }

    @ObservationIgnored var validationComponents: [String: @MainActor (Element, LiveProgram, String?) -> JSONValue?] = [:]

    /// Resolve the enclosing form from today's tree, not a retained view value.
    func enclosingForm(of element: Element) -> Element? {
        guard let origin = element.origin, origin.program == id else { return nil }
        for end in origin.steps.indices.reversed() {
            if case let .element(form)? = locate(Array(origin.steps.prefix(end)))?.value,
               form.typeName == "Form" { return form }
        }
        return nil
    }

    static let stale = LiveError(description: "the program changed since this was drawn, so nothing was done")
    static let streaming = LiveError(description: "a response is streaming, so nothing was done")

    /// Runs the action in `element`'s prop `prop`, as a click on it does: a
    /// plan runs its steps, an older `{type, …}` action object becomes one
    /// host event, and no action at all asks to continue the conversation
    /// with `message`. Host events go to `onHostEvent`, after the redraw.
    /// `form` is the form the action sits in; its fields go with the events.
    ///
    /// Only an element of this program runs anything, and only while what
    /// was drawn from it is still there: one from the current evaluation, or
    /// one from an earlier evaluation that is exactly the element now at its
    /// place, in a form of the same name (see `current(_:form:)`). Otherwise
    /// it throws and nothing happens, so a click runs exactly the action that
    /// was on screen, or none. While `isStreaming`, it throws too.
    public func run(_ prop: String, of element: Element, message: String, form: String? = nil) throws {
        guard !isStreaming else { throw Self.streaming }
        guard let element = current(element, form: form), let origin = element.origin else { throw Self.stale }
        guard let found = locate(origin.steps + [prop]) else {
            throw LiveError(description: "nothing at \(origin.path) in the tree")
        }
        let jsonPath = found.jsonPath
        let output: ActionOutput
        switch found.value {
        case nil, .scalar(.null)?:
            output = try session.run(action: nil, message: message, form: form)
        case .plan?:
            guard let handle else { throw LiveError(description: "no evaluated tree") }
            let action = try session.actionAt(evaluation: handle, path: jsonPath)
            output = try session.run(action: action, message: message, form: form)
        case let .record(fields)?:
            output = try session.runLegacy(json: PropValue.record(fields).json.compact, message: message, form: form)
        case let other?:
            throw LiveError(description: "\(prop) holds \(other.summary), not an action")
        }
        try finish(output)
    }

    /// Runs a table's action cell, as a click on it does: like a button with
    /// no action, it asks to continue the conversation with the cell's text.
    /// The same checks as `run`: the table must still be what was drawn.
    func runActionCell(row: Int, column: Int, of table: Element, form: String?) throws {
        guard !isStreaming else { throw Self.streaming }
        guard let table = current(table, form: form), let origin = table.origin,
              case let .scalar(.string(label))? = locate(origin.steps + ["rows", String(row), String(column)])?.value
        else { throw Self.stale }
        try finish(try session.run(action: nil, message: label, form: form))
    }

    /// Ends the mutation a host event asked for: with `succeeded`, the plan
    /// carries on from the step after it; otherwise it stops there. Events
    /// that don't wait on a mutation are ignored.
    public func resume(_ event: HostEvent, succeeded: Bool) throws {
        guard let continuation = event.continuation else { return }
        try finish(try session.resume(continuation: continuation, succeeded: succeeded))
    }

    /// Redraws, then hands over the events. The store may have changed even
    /// when redrawing fails, so the events (and a mutation's continuation)
    /// are delivered regardless, and the redraw's failure is thrown after.
    /// Internal so the package's tests can make the redraw fail.
    func finish(_ output: ActionOutput) throws {
        var redrawError: (any Error)?
        var projectionWarning: String?
        do { projectionWarning = try refresh() } catch { redrawError = error }
        let stepFailure = output.failure.map { failure in
            (failure.step.map { "Step \($0): " } ?? "") + Self.describe(failure.error)
        }
        failure = [stepFailure, projectionWarning, redrawError.map(Self.describe)].compactMap { $0 }.joined(separator: "\n")
        if failure == "" { failure = nil }
        var events = output.events.map { json in
            HostEvent(json: (try? JSONValue.parse(json)) ?? .object([("type", .string("")), ("unreadable", .string(json))]))
        }
        if let continuation = output.continuation, !events.isEmpty {
            events[events.count - 1].continuation = continuation
        }
        for event in events { onHostEvent(event) }
        if let redrawError { throw redrawError }
    }

    /// For built-in views: does `body`, keeping why it failed.
    func attempt(_ body: () throws -> Void) {
        failure = nil
        do {
            try body()
        } catch {
            failure = Self.describe(error)
        }
    }

    // MARK: Evaluating

    /// Evaluates the tree again if the store has changed since it last was.
    @discardableResult private func refresh() throws -> String? {
        pristineCache = nil
        let base = completeDisplay ?? display
        guard try session.version() != base.version else {
            if let previewSession {
                do {
                    if try previewSession.version() != version {
                        display = try evaluated(previewSession, from: display, streaming: true)
                    }
                } catch { return "preview projection: " + Self.describe(error) }
            }
            return nil
        }
        let next: Display
        if hasProgram {
            next = try evaluated(session, from: base)
        } else {
            var store = base
            store.version = try session.version()
            store.state = try JSONValue.parse(session.snapshot())
            next = store
        }
        if let previewSession {
            completeDisplay = next
            do {
                // Remove values explicitly made undefined in the complete store.
                if case let .object(old) = base.state {
                    for (name, _) in old where next.state[name] == nil {
                        _ = try previewSession.set(name: name, json: nil)
                    }
                }
                let warning = try copyEditedState(to: previewSession, from: next)
                display = try evaluated(previewSession, from: display, streaming: true)
                return warning
            } catch {
                // Keep the existing gated preview drawing and the new actual
                // complete snapshot. A later discard/finish exposes that store.
                return "preview projection: " + Self.describe(error)
            }
        } else { display = next }
        return nil
    }

    private func evaluated(_ runtime: RuntimeSession, from base: Display,
                           programID: UUID? = nil, streaming: Bool? = nil) throws -> Display {
        let started = ContinuousClock.now
        let output = try runtime.evaluate()
        let json = try JSONValue.parse(output.json)
        guard base.generation < UInt64.max else { throw LiveError(description: "evaluation generation exhausted") }
        let nextGeneration = base.generation + 1
        let nextID = programID ?? base.id
        let origin = Origin(program: nextID, generation: nextGeneration, steps: [])
        let nextRootJSON = json["root"]
        let nextRoot = try nextRootJSON.flatMap { $0 == .null ? nil : try Element(json: $0, at: origin) }
        let nextErrors = (json["errors"]?.array ?? []).map(RuntimeIssue.init(json:))
        let nextVersion = try runtime.version()
        let nextState = try JSONValue.parse(runtime.snapshot())
        // Publish only after every fallible Swift conversion/read succeeded.
        return Display(root: nextRoot, errors: nextErrors, state: nextState, version: nextVersion,
            sizeLimited: output.sizeLimited, depthLimited: output.depthLimited,
            evaluation: ContinuousClock.now - started, rootJSON: nextRootJSON, handle: output.evaluation,
            id: nextID, generation: nextGeneration, streaming: streaming ?? base.streaming)
    }

    private struct Located {
        /// The value; nil for a prop the element doesn't have.
        var value: PropValue?
        /// Its path in the runtime's JSON: `props` before each prop name.
        var jsonPath: [String]
        /// The name of the innermost Form above it, as a Form's view names it.
        var form: String?
    }

    /// What's at the end of `parts` (prop names, list indices and record keys
    /// from the root); nil when the path leads nowhere.
    private func locate(_ parts: [String]) -> Located? {
        guard let root else { return nil }
        var current = PropValue.element(root)
        var jsonPath: [String] = []
        var form: String?
        for (i, part) in parts.enumerated() {
            switch current {
            case let .element(element):
                jsonPath += ["props", part]
                if element.typeName == "Form" { form = try? PropReader(element: element).text("name").plain }
                guard let prop = element.props.first(where: { $0.name == part }) else {
                    return i == parts.count - 1 ? Located(value: nil, jsonPath: jsonPath, form: form) : nil
                }
                current = prop.value
            case let .list(items):
                guard let index = Int(part), items.indices.contains(index) else { return nil }
                jsonPath.append(String(index))
                current = items[index]
            case let .record(fields):
                guard let field = fields.first(where: { $0.name == part }) else { return nil }
                jsonPath.append(part)
                current = field.value
            default:
                return nil
            }
        }
        return Located(value: current, jsonPath: jsonPath, form: form)
    }

    static func describe(_ error: any Error) -> String {
        guard let error = error as? OpenUiError else { return String(describing: error) }
        switch error {
        case let .RefusedWrite(reason), let .Evaluation(reason), let .LimitExceeded(reason),
             let .InvalidJson(reason), let .Worker(reason):
            return reason
        default:
            return String(describing: error)
        }
    }
}

/// A runtime error from evaluating the tree, such as a prop whose expression
/// failed (it keeps its unevaluated value) or a program with no root.
public struct RuntimeIssue: Sendable, Equatable {
    public var code: String
    public var component: String?
    public var statementId: String?
    /// Upstream's wording, naming the prop.
    public var message: String
    public var hint: String?

    init(json: JSONValue) {
        code = json["code"]?.string ?? ""
        component = json["component"]?.string
        statementId = json["statementId"]?.string
        message = json["message"]?.string ?? ""
        hint = json["hint"]?.string
    }
}

/// Something a program asks the app to do, in the shape upstream's renderer
/// hands its action handler.
public struct HostEvent: Sendable {
    /// `continue_conversation`, `open_url`, a custom action's name, or a
    /// query's or mutation's run.
    public var type: String
    public var params: JSONValue
    /// The message for the assistant, such as the clicked button's label.
    public var humanFriendlyMessage: String
    /// The form the action sat in, if any.
    public var formName: String?
    /// That form's fields, or the whole store outside a form.
    public var formState: JSONValue
    /// The event as the runtime gave it.
    public var json: JSONValue
    /// For a mutation, the plan waiting on it; see `LiveProgram.resume`.
    var continuation: ContinuationHandle?

    init(json: JSONValue) {
        type = json["type"]?.string ?? ""
        params = json["params"] ?? .object([])
        humanFriendlyMessage = json["humanFriendlyMessage"]?.string ?? ""
        formName = json["formName"]?.string
        formState = json["formState"] ?? .null
        self.json = json
    }

    /// Whether the plan that sent it waits for `LiveProgram.resume`.
    public var awaitsResume: Bool { continuation != nil }
}

public struct LiveError: Error, CustomStringConvertible {
    public let description: String
}
