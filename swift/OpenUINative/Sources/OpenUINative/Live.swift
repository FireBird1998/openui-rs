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
    /// What the queries are doing: loading, refetching and their errors.
    /// Empty without a tool provider (see `attachToolProvider`).
    public var queries: QueryStatus { display.queries }
    /// Message metadata and the response bundle error for the drawing on screen.
    public var response: ResponseMetadata? { display.step?.response }
    /// The session's parser diagnostic for an invalid response bundle.
    public var parserError: JSONValue? { display.parserError }
    /// The session's loading indicator, including a query waiting for a streaming bundle.
    public var isQueryLoading: Bool { display.queryLoading }
    /// A response is still streaming into the program. The app sets it while
    /// the program's text is arriving and clears it once the response is
    /// complete. Meanwhile built-in buttons, inputs and selects are disabled,
    /// and a click that still arrives is ignored. An explicit `run` on a
    /// bare streaming program refuses. A stream push remains
    /// gated until a finish result is shown, even if this flag is cleared.
    /// `write` refuses too; app state writes with `set` and `resume` still work.
    /// The request and the preview phase gate new query registrations.
    /// Previously registered calls and refresh intervals stay live.
    public var isStreaming: Bool {
        get { requestedStreaming || stepIsStreaming || !interactionSuspensions.isEmpty }
        set {
            guard newValue != requestedStreaming else { return }
            requestedStreaming = newValue
            if !showingPreview { queryStreamingChanged() }
        }
    }
    /// The response on screen has not settled: it is streaming, or it is a
    /// response bundle that is incomplete or has an invalid scripts section
    /// (`parserError`). Upstream hands this `blocked` to its components as
    /// `isStreaming`; built-in buttons, inputs and selects are disabled
    /// while it is true.
    public var isBlocked: Bool { isStreaming || bundleBlocksActions }
    /// Built-in controls can interact only with a complete runtime.
    public var acceptsInteractions: Bool { !isBlocked }
    /// The session's `blocked` for the drawing on screen. Only a response
    /// bundle's counts here: a bare program's is the streaming request at its
    /// draw, which `isStreaming` already reports as it is now.
    private var bundleBlocksActions: Bool { response?.state.isBundle == true && display.blocked }
    /// The app-owned request, separate from an immutable preview step's phase.
    /// Temporary hosts use suspendInteractions(), so they never overwrite
    /// an app request made while their asynchronous work is suspended.
    public var isStreamingRequested: Bool { requestedStreaming }
    private var requestedStreaming = false
    private var stepIsStreaming: Bool { display.streaming }
    var showingPreview: Bool { previewSession != nil }
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
    @ObservationIgnored let library: Library
    @ObservationIgnored private let promptSpec: String?
    @ObservationIgnored private let bindings: [(component: String, prop: String)]
    @ObservationIgnored private var previewSession: RuntimeSession?
    @ObservationIgnored private var completeDisplay: Display?
    @ObservationIgnored private let rootName: String?
    @ObservationIgnored private lazy var responseParser = Task {
        try await library.core.session(library: library, root: rootName)
    }
    @ObservationIgnored private var pendingResponse: UUID?
    /// The pristine state variables at a store version, read once.
    @ObservationIgnored private var pristineCache: (version: UInt64, names: Set<String>)?
    private var handle: EvaluationHandle? { display.handle }
    @ObservationIgnored private var hasProgram = false
    /// Performs the session's tool calls while a tool provider is attached.
    private(set) var queryDriver: QueryDriver?
    private var completeGeneration: UInt64 { (completeDisplay ?? display).generation }
    /// Queries are paused by the app, or because every view drawing this
    /// program has disappeared.
    @ObservationIgnored private var queriesPausedByApp = false
    @ObservationIgnored private var visibleViews = 0
    @ObservationIgnored private var hiddenByViews = false
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
        var step: Step?
        var parserError: JSONValue?
        var queryLoading = false
        /// The session's `blocked` flag from the evaluation on screen.
        var blocked = false
        var streaming = false
        var queries = QueryStatus()
    }
    private var display = Display()

    /// A session for programs in `library`. The props that are bindings come
    /// from `promptSpec` (its signatures' `$binding<…>` props; the library
    /// object or the whole spec) and from `bindings`, for components the spec
    /// doesn't describe.
    public init(
        library: Library,
        root: String? = nil,
        promptSpec: String? = nil,
        bindings: [(component: String, prop: String)] = [],
        onHostEvent: @escaping @MainActor (HostEvent) -> Void = { _ in }
    ) throws(OpenUIError) {
        self.rootName = root
        self.library = library
        self.promptSpec = promptSpec
        self.bindings = bindings
        let session = try bridged {
            let session = try RuntimeSession(library: library.inner, promptSpec: promptSpec)
            for binding in bindings {
                try session.markBinding(component: binding.component, prop: binding.prop)
            }
            return session
        }
        self.session = session
        self.onHostEvent = onHostEvent
    }

    /// Reads and shows the whole raw response so far, including edits.
    /// A newer show or discard supersedes a read that is still in progress.
    @discardableResult
    public func show(response text: String, streaming: Bool) async throws(OpenUIError) -> Step {
        let request = UUID()
        pendingResponse = request
        let step: Step
        do {
            let parser = try await responseParser.value
            step = try await parser.response(text, streaming: streaming)
            try Task.checkCancellation()
        } catch { throw OpenUIError(error) }
        guard request == pendingResponse else {
            throw OpenUIError(.cancelled, "a newer show or discard superseded this read")
        }
        try show(step)
        return step
    }

    /// Shows a parse result: the store takes its state declarations (a value
    /// set before survives; a declared starting value fills in or replaces
    /// one still pristine), `persisted` values win over both, and the tree is
    /// evaluated. Every accepted complete show revokes older elements and
    /// markers, even for identical programs. A stream push uses a separate,
    /// noninteractive preview session; `discardPreview()` restores the complete
    /// session after a parser failure. Call with each step of a stream. Program
    /// replacement is atomic, including initialization, evaluation and native
    /// display admission. A refused show preserves both sessions and drawings.
    public func show(_ step: Step, persisted: JSONValue? = nil) throws(OpenUIError) {
        let oldDisplay = display
        let oldCompleteDisplay = completeDisplay
        pendingResponse = nil
        let oldPreview = previewSession
        let oldCache = pristineCache
        let oldFailure = failure
        do {
            try queryDriver?.advanceClock()
            try apply(step, persisted: persisted)
            let bundleChanged = (oldDisplay.step?.response?.state.isBundle == true) != (step.response?.state.isBundle == true)
            if step.isStreaming && ((oldPreview == nil && !requestedStreaming) || bundleChanged) {
                queryStreamingChanged()
            } else {
                syncQueries(alreadyDrawn: !step.isStreaming)
            }
        } catch {
            if previewSession !== oldPreview { previewSession?.dispose() }
            if let oldPreview, previewSession === oldPreview, hasToolProvider,
               step.response?.state.isBundle == true || oldDisplay.step?.response?.state.isBundle == true {
                // A refused replacement can follow a successful projection.
                // Restore the references used by the still-displayed preview.
                try? projectQueryValues(to: oldPreview, for: oldDisplay.step, from: oldCompleteDisplay ?? oldDisplay)
            }
            previewSession = oldPreview
            completeDisplay = oldCompleteDisplay
            display = oldDisplay
            pristineCache = oldCache
            failure = oldFailure
            syncQueries()
            throw OpenUIError(error)
        }
    }

    private func apply(_ step: Step, persisted: JSONValue?) throws {
        if step.isStreaming {
            try showPreview(step, persisted: persisted)
            return
        }
        guard generation < UInt64.max else { throw Self.generationsExhausted }
        let base = completeDisplay ?? display
        guard base.generation < UInt64.max else { throw Self.generationsExhausted }
        let started = ContinuousClock.now
        let replaced = try session.replaceProgramWithResponse(program: step.parsed, persisted: persisted?.compact,
            projected: nil, nativeView: true, responseState: state(for: step, streaming: requestedStreaming))
        let initialized = replaced.initialization
        let next = try decoded(replaced.evaluation, snapshot: replaced.snapshot, version: replaced.version,
            from: base, step: step, programID: UUID(), streaming: false, started: started)
        pristineCache = nil
        previewSession?.dispose()
        previewSession = nil
        completeDisplay = nil
        display = next
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
        pendingResponse = nil
        guard showingPreview else { return }
        let wasBundle = response?.state.isBundle == true
        previewSession?.dispose()
        previewSession = nil
        pristineCache = nil
        if let completeDisplay { display = completeDisplay }
        completeDisplay = nil
        if !requestedStreaming || wasBundle != (response?.state.isBundle == true) { queryStreamingChanged() }
    }

    private func showPreview(_ step: Step, persisted: JSONValue?) throws {
        do {
            if previewSession == nil {
                let preview = try RuntimeSession(library: library.inner, promptSpec: promptSpec)
                for binding in bindings { try preview.markBinding(component: binding.component, prop: binding.prop) }
                // Keep the actual complete session. Only the noninteractive
                // drawing uses a JSON projection; copied plans gain no authority.
                completeDisplay = display
                previewSession = preview
                if let completeStep = display.step {
                    try preview.setProgram(program: completeStep.parsed)
                    _ = try preview.initialize(persisted: nil)
                }
            }
            guard let previewSession else { throw OpenUIError(.unexpected, "no preview session") }
            guard display.generation < UInt64.max else { throw Self.generationsExhausted }
            let projected = try editedState(from: completeDisplay ?? display)
            let started = ContinuousClock.now
            if step.response?.state.isBundle == true || response?.state.isBundle == true {
                try projectQueryValues(to: previewSession, for: step, from: completeDisplay ?? display)
            }
            let replaced = try previewSession.replaceProgramWithResponse(program: step.parsed, persisted: persisted?.compact,
                projected: projected?.compact, nativeView: true, responseState: state(for: step, streaming: true))
            let initialized = replaced.initialization
            let projectionWarning = projectionWarning(replaced.projection)
            let next = try decoded(replaced.evaluation, snapshot: replaced.snapshot, version: replaced.version,
                from: display, step: step, programID: UUID(), streaming: true, started: started)
            pristineCache = nil
            display = next
            let initializationWarning = initialized.refused.isEmpty ? nil : initialized.refused.map {
                "\($0.name): \(Self.describe($0.error))"
            }.joined(separator: "\n")
            let warnings = [projectionWarning, initializationWarning].compactMap { $0 }
            failure = warnings.isEmpty ? nil : warnings.joined(separator: "\n")
        }
    }

    private func state(for step: Step?, streaming: Bool) -> ResponseState {
        var state = step?.response?.state ?? ResponseState()
        state.streaming = streaming
        return state
    }

    /// A preview owns no calls. Resolve its bundle's statements against the
    /// attached provider's last snapshot. Newly declared ids read as null.
    private func projectQueryValues(to preview: RuntimeSession, for step: Step?, from complete: Display) throws {
        guard hasToolProvider, let step, step.response?.state.isBundle == true else {
            try preview.setReferenceValues(json: "{}")
            return
        }
        let statements = (step.result.json["queryStatements"]?.array ?? []) + (step.result.json["mutationStatements"]?.array ?? [])
        guard case let .object(values) = complete.queries.values else { return }
        let snapshot = Dictionary(values, uniquingKeysWith: { _, last in last })
        let entries = statements.compactMap { statement -> (String, JSONValue)? in
            guard let id = statement["statementId"]?.string else { return nil }
            return (id, snapshot[id] ?? .null)
        }
        try preview.setReferenceValues(json: JSONValue.object(entries).compact)
    }

    private func copyEditedState(to preview: RuntimeSession, from complete: Display) throws -> String? {
        let initialized = try preview.initialize(persisted: editedState(from: complete)?.compact)
        return projectionWarning(initialized)
    }

    private func editedState(from complete: Display) throws -> JSONValue? {
        guard case let .object(entries) = complete.state else { return nil }
        return .object(try entries.filter { try !session.isPristine(name: $0.key) })
    }

    private func projectionWarning(_ initialized: InitializationOutput) -> String? {
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
    public func set(_ name: String, to value: JSONValue?) throws(OpenUIError) {
        try bridged {
            _ = try onQueryClock { try $0.set(name: name, json: value?.compact) }
            let warning = mirror { _ = try $0.set(name: name, json: value?.compact) }
            let redrawWarning = try refresh()
            if let warning = warning ?? redrawWarning { failure = warning }
        }
    }

    /// Writes a value through a binding marker from the tree: the marker's
    /// expression turns it into what's stored. A marker from this program's
    /// current evaluation writes. One from an earlier evaluation (a second
    /// keystroke that arrived before the redraw) writes only while the same
    /// marker is at its place in the tree now; otherwise it throws and
    /// changes nothing.
    public func write(_ marker: BindingMarker, _ value: JSONValue) throws(OpenUIError) {
        guard acceptsInteractions else { throw interactionError }
        guard let origin = marker.origin, origin.program == id else { throw Self.stale }
        if origin.generation != generation {
            guard case let .binding(now)? = locate(origin.steps)?.value, now.json == marker.json else { throw Self.stale }
        }
        try bridged {
            _ = try onQueryClock { try $0.writeBinding(marker: marker.json.compact, json: value.compact) }
            try refresh()
        }
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
    public func setField(form: String?, componentType: String? = nil, name: String, to value: JSONValue?) throws(OpenUIError) {
        try bridged {
            _ = try onQueryClock { try $0.setField(form: form, componentType: componentType, name: name, json: value?.compact, save: true) }
            let warning = mirror {
                _ = try $0.setField(form: form, componentType: componentType, name: name, json: value?.compact, save: true)
            }
            let redrawWarning = try refresh()
            if let warning = warning ?? redrawWarning { failure = warning }
        }
    }

    /// Writes what a person typed or picked in a built-in form control
    /// drawn from `element`: through its binding marker, or else into its
    /// form field. Refused, changing nothing, when what was drawn is gone
    /// (see `current(_:form:)`), or while streaming. Entering the value already
    /// held is a no-op, including when the binding transforms new values.
    /// Chips/OptionCards include their component type and accept nil when a
    /// single selection is cleared; a binding refuses undefined (`.refusedWrite`).
    public func enter(_ value: JSONValue?, into element: Element, form: String? = nil) throws(OpenUIError) {
        guard acceptsInteractions else { throw interactionError }
        guard let element = current(element, form: form) else { throw Self.stale }
        let props = PropReader(element: element)
        let name: String
        do { name = try props.text("name").plain } catch {
            throw OpenUIError(.unusableElement, "\(element.typeName) has no name to enter a value under")
        }
        let field = FieldValue(props, name: name)
        guard value != field.value(in: self, form: form, includingCompositeDefaults: true) else { return }
        if case let .binding(marker)? = props.value("value") {
            guard let value else { throw OpenUIError(.refusedWrite, "a binding cannot be set to undefined") }
            try write(marker, value)
        } else {
            let type = SelectionField.isSelection(element) ? element.typeName : nil
            try setField(form: form, componentType: type, name: name, to: value)
        }
    }

    /// Hydrates only a missing selection default, never while streaming or
    /// from an obsolete view. The runtime owns the undefined/default guard.
    func initializeSelection(_ element: Element, form: String?) throws {
        guard acceptsInteractions, let element = current(element, form: form) else { return }
        let props = PropReader(element: element)
        guard let value = SelectionField.defaultValue(props) else { return }
        try onQueryClock {
            try $0.initializeField(form: form, componentType: element.typeName,
                name: try props.text("name").plain, json: value.compact, streaming: false)
        }
        try refresh()
    }

    // MARK: Actions

    /// Whether `element` is from this program's current evaluation: what's
    /// drawn from it is what the tree holds now.
    public func isCurrent(_ element: Element) -> Bool {
        guard let origin = element.origin else { return false }
        return origin.program == id && origin.generation == generation
    }

    /// What a click or a write from `element`, drawn inside `form`, acts on.
    /// While `element` is from the current evaluation, itself. From an earlier
    /// evaluation of this program (an event that arrived before the redraw),
    /// the element now at the same place, but only if it is exactly what was
    /// drawn (the same component, props, actions and markers) inside a form
    /// of the same name. Otherwise nil: what was drawn there is gone.
    func current(_ element: Element, form: String?) -> Element? {
        guard let origin = element.origin, origin.program == id else { return nil }
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

    static let stale = OpenUIError(.stale, "the program changed since this was drawn, so nothing was done")
    /// Why an interaction is refused while `acceptsInteractions` is false:
    /// the response bundle's error when it has one, otherwise streaming.
    private var interactionError: OpenUIError {
        response?.error.map { OpenUIError(.blocked, $0) } ?? Self.streaming
    }

    static let streaming = OpenUIError(.streaming, "a response is streaming, so nothing was done")
    static let generationsExhausted = OpenUIError(.limitExceeded, "evaluation generation exhausted")

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
    /// was on screen. A blocked response bundle ignores the action. A bare
    /// streaming preview retains its existing refusal.
    public func run(_ prop: String, of element: Element, message: String, form: String? = nil) throws(OpenUIError) {
        guard !bundleBlocksActions else { return }
        guard !isStreaming else { throw Self.streaming }
        guard let element = current(element, form: form), let origin = element.origin else { throw Self.stale }
        guard let found = locate(origin.steps + [prop]) else {
            throw OpenUIError(.invalidHandle, "nothing at \(origin.path) in the tree")
        }
        let jsonPath = found.jsonPath
        try bridged {
            let output: ActionOutput
            switch found.value {
            case nil, .scalar(.null)?:
                output = try onQueryClock { try $0.run(action: nil, message: message, form: form) }
            case .plan?:
                guard let handle else { throw OpenUIError(.invalidHandle, "no evaluated tree") }
                let action = try session.actionAt(evaluation: handle, path: jsonPath)
                output = try onQueryClock { try $0.run(action: action, message: message, form: form) }
            case let .record(fields)?:
                output = try onQueryClock { try $0.runLegacy(json: PropValue.record(fields).json.compact, message: message, form: form) }
            case let other?:
                throw OpenUIError(.invalidHandle, "\(prop) holds \(other.summary), not an action")
            }
            try finish(output)
        }
    }

    /// Runs a table's action cell, as a click on it does: like a button with
    /// no action, it asks to continue the conversation with the cell's text.
    /// The same checks as `run`: the table must still be what was drawn.
    func runActionCell(row: Int, column: Int, of table: Element, form: String?) throws {
        guard !bundleBlocksActions else { return }
        guard !isStreaming else { throw Self.streaming }
        guard let table = current(table, form: form), let origin = table.origin,
              case let .scalar(.string(label))? = locate(origin.steps + ["rows", String(row), String(column)])?.value
        else { throw Self.stale }
        try finish(try onQueryClock { try $0.run(action: nil, message: label, form: form) })
    }

    /// Ends the mutation a host event asked for: with `succeeded`, the plan
    /// carries on from the step after it; otherwise it stops there. Events
    /// that don't wait on a mutation are ignored.
    public func resume(_ event: HostEvent, succeeded: Bool) throws(OpenUIError) {
        guard let continuation = event.continuation else { return }
        try bridged { try finish(try onQueryClock { try $0.resume(continuation: continuation, succeeded: succeeded) }) }
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
        for event in Self.hostEvents(output) { onHostEvent(event) }
        if let redrawError { throw redrawError }
    }

    static func hostEvents(_ output: ActionOutput) -> [HostEvent] {
        var events = output.events.map { json in
            HostEvent(json: (try? JSONValue.parse(json)) ?? .object([("type", .string("")), ("unreadable", .string(json))]))
        }
        if let continuation = output.continuation, !events.isEmpty {
            events[events.count - 1].continuation = continuation
        }
        return events
    }

    // MARK: Queries

    /// Sample the clock before commands that can register a new interval.
    /// Even a refused command can follow a tick that needs to be dispatched.
    @discardableResult private func onQueryClock<T>(_ command: (RuntimeSession) throws -> T) throws -> T {
        try queryDriver?.advanceClock()
        do { return try command(session) }
        catch {
            syncQueries()
            throw error
        }
    }

    /// Takes the session's query update: redraws if the queries changed and
    /// the tree doesn't show it yet, hands the wanted calls and the refresh
    /// deadline to the driver, then delivers the host events of the action
    /// plans that resumed after a mutation, as a click's are delivered.
    func syncQueries(alreadyDrawn: Bool = false) {
        guard let driver = queryDriver else { return }
        var effects: QueryEffects
        do { effects = try session.takeQueryUpdate() } catch { queryFailure(error); return }
        var redrawWarning: String?
        // A draw evaluates the queries before drawing, so a change taken
        // after one is already on screen.
        if effects.changed && !alreadyDrawn && hasProgram {
            do {
                redrawWarning = try refreshStore(force: true)
                let more = try session.takeQueryUpdate()
                effects.calls += more.calls
                effects.actions += more.actions
                effects.readerRoundsLimited = effects.readerRoundsLimited || more.readerRoundsLimited
                effects.evaluationError = more.evaluationError
                effects.nextRefreshDueMs = more.nextRefreshDueMs
            } catch { redrawWarning = Self.describe(error) }
        }
        if effects.readerRoundsLimited {
            let message = [effects.evaluationError.map(Self.describe),
                           "Query readers were still changing at the reader round limit."].compactMap { $0 }.joined(separator: "\n")
            display.queries.evaluationError = message
            completeDisplay?.queries.evaluationError = message
        }
        // Calls taken are dispatched even when redrawing failed: the session
        // hands each out once.
        driver.dispatch(effects.calls, deadline: effects.nextRefreshDueMs)
        deliverQueryActions(effects.actions, redrawWarning: redrawWarning)
    }

    func deliverQueryActions(_ actions: [ActionOutput], redrawWarning: String?) {
        let failures = actions.compactMap { output in
            output.failure.map { ($0.step.map { "Step \($0): " } ?? "") + Self.describe($0.error) }
        } + [redrawWarning].compactMap { $0 }
        if !failures.isEmpty { failure = failures.joined(separator: "\n") }
        for output in actions {
            for event in Self.hostEvents(output) { onHostEvent(event) }
        }
    }

    /// An error with no caller to throw to: a tool call's outcome or a timer.
    func queryFailure(_ error: any Error) {
        failure = Self.describe(error)
    }

    private func queryStreamingChanged() {
        // A provider-free response bundle also has a session action gate.
        guard hasProgram, hasToolProvider || (!showingPreview && response?.state.isBundle == true) else { return }
        do {
            try queryDriver?.advanceClock()
            if let warning = try refresh(force: true) { failure = warning }
        } catch { queryFailure(error) }
    }

    /// Lets the queries run: called after attaching, and by the view lifecycle.
    private func applyQueryPause() throws {
        let paused = queriesPausedByApp || hiddenByViews
        guard let driver = queryDriver, driver.paused != paused else {
            queriesPaused = paused
            return
        }
        if paused { try driver.pause() } else { try driver.resume() }
        queriesPaused = paused
        syncQueries()
    }

    /// A tool provider performs the program's queries and mutations: the
    /// session decides when to call which tool, and this live program calls
    /// `provider`, reports each outcome and redraws. A settled program
    /// registers its queries. Existing calls and refresh intervals can carry
    /// on during a later stream. `queries` says what is loading and what
    /// failed. A mutation's
    /// `@Run` waits for its call and the plan carries on after it, delivering
    /// its host events to `onHostEvent`. A query's refresh interval is kept by
    /// a timer on `clock`, a monotonic clock that tests can replace. The
    /// session raises any shorter interval to `minimumRefreshInterval`, a whole
    /// number of milliseconds from 1 ms to about 24 days.
    ///
    /// Nothing times a call out. Calls in flight are cancelled, and their
    /// outcomes ignored, when the live program goes away, when queries pause
    /// and on `detachToolProvider()`.
    public func attachToolProvider<C: Clock>(
        clock: C,
        minimumRefreshInterval: Duration = .seconds(1),
        _ provider: @escaping ToolProvider
    ) throws(OpenUIError) where C.Duration == Duration {
        guard queryDriver == nil else { throw OpenUIError(.toolProviderAttached, "a tool provider is already attached") }
        let minimum = QueryDriver.milliseconds(minimumRefreshInterval)
        guard minimumRefreshInterval == .milliseconds(minimum) else {
            throw OpenUIError(.invalidArgument, "the minimum refresh interval must be a whole number of milliseconds")
        }
        let driver = QueryDriver(program: self, session: session, clock: clock, provider: provider)
        try bridged { try session.attachToolProviderAt(nowMs: driver.now(), minimumMs: minimum) }
        queryDriver = driver
        do {
            if queriesPaused { try driver.pause() }
            if hasProgram || showingPreview { try refreshStore(force: true) }
        } catch {
            queryDriver = nil
            driver.stop()
            try? session.detachToolProvider()
            _ = try? refreshStore(force: true)
            throw OpenUIError(error)
        }
        syncQueries(alreadyDrawn: true)
    }

    /// `attachToolProvider(clock:minimumRefreshInterval:_:)` on the
    /// continuous clock, which keeps counting while the Mac sleeps.
    public func attachToolProvider(minimumRefreshInterval: Duration = .seconds(1), _ provider: @escaping ToolProvider) throws(OpenUIError) {
        try attachToolProvider(clock: ContinuousClock(), minimumRefreshInterval: minimumRefreshInterval, provider)
    }

    /// Cancels the calls in flight, ignoring their outcomes, and detaches
    /// the provider: the session forgets the queries' results and the program
    /// redraws as it would without one.
    public func detachToolProvider() throws(OpenUIError) {
        guard let driver = queryDriver else { return }
        try bridged { try session.detachToolProvider() }
        driver.stop()
        queryDriver = nil
        try bridged { try refreshStore(force: true) }
    }

    /// Whether a tool provider is attached.
    public var hasToolProvider: Bool { queryDriver != nil }

    /// Runs again every query that has an error, as upstream's Retry button
    /// does (with no errors, every query).
    public func retryFailedQueries() throws(OpenUIError) {
        guard queryDriver != nil else { return }
        try bridged { try onQueryClock { try $0.retryFailedQueries() } }
        syncQueries()
    }

    /// Whether queries are paused, by `pauseQueries()` or because every view
    /// drawing this program has disappeared. While paused no refresh fires,
    /// calls in flight are cancelled and results already loaded stay.
    public private(set) var queriesPaused = false

    /// Pauses the queries until `resumeQueries()`, as when the app goes to
    /// the background. A view appearing does not resume them.
    public func pauseQueries() throws(OpenUIError) {
        queriesPausedByApp = true
        try bridged { try applyQueryPause() }
    }

    /// Undoes `pauseQueries()`. Queries resume unless every view drawing this
    /// program has disappeared; they fetch what they still need and refresh.
    public func resumeQueries() throws(OpenUIError) {
        queriesPausedByApp = false
        try bridged { try applyQueryPause() }
    }

    /// The view lifecycle: queries pause when the last `ElementView(live:)`
    /// drawing this program disappears, and resume when one appears.
    func viewAppeared() {
        visibleViews += 1
        guard hiddenByViews else { return }
        hiddenByViews = false
        do { try applyQueryPause() } catch { queryFailure(error) }
    }

    func viewDisappeared() {
        visibleViews = max(0, visibleViews - 1)
        guard visibleViews == 0, !hiddenByViews else { return }
        hiddenByViews = true
        do { try applyQueryPause() } catch { queryFailure(error) }
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

    /// Evaluates the tree again if the store has changed since it last was,
    /// then takes the session's query update.
    @discardableResult private func refresh(force: Bool = false) throws -> String? {
        let before = completeGeneration
        defer { syncQueries(alreadyDrawn: completeGeneration != before) }
        return try refreshStore(force: force)
    }

    /// Evaluates the tree again if the store has changed since it last was,
    /// or always with `force` (the queries' results changed).
    @discardableResult private func refreshStore(force: Bool = false) throws -> String? {
        pristineCache = nil
        let base = completeDisplay ?? display
        guard try force || session.version() != base.version else {
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
            display.queries = next.queries
            do {
                // Remove values explicitly made undefined in the complete store.
                if case let .object(old) = base.state {
                    for (name, _) in old where next.state[name] == nil {
                        _ = try previewSession.set(name: name, json: nil)
                    }
                }
                if response?.state.isBundle == true { try projectQueryValues(to: previewSession, for: display.step, from: next) }
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
        // With no provider the hidden complete session has no query work to
        // gate. Keep its actions and loading flag ready for discardPreview.
        let gateForPreview = showingPreview && (runtime !== session || hasToolProvider)
        var responseState = state(for: base.step, streaming: requestedStreaming || gateForPreview)
        if runtime === session && gateForPreview {
            // Retry belongs to the visible response. The retained complete
            // drawing keeps its own diagnostic and its last settled scripts.
            responseState.isBundle = response?.state.isBundle ?? false
        }
        let output = try runtime.evaluateWithResponse(responseState: responseState)
        return try decoded(output, snapshot: runtime.snapshot(), version: runtime.version(),
            from: base, programID: programID, streaming: streaming, started: started)
    }

    /// Replacement supplies serialized display data and enforces this decoder's
    /// root shape and depth before committing. No session reads follow commit.
    private func decoded(_ output: EvaluationOutput, snapshot: String, version: UInt64,
                         from base: Display, step: Step? = nil, programID: UUID?, streaming: Bool?, started: ContinuousClock.Instant) throws -> Display {
        let json = try JSONValue.parse(output.json)
        guard base.generation < UInt64.max else { throw Self.generationsExhausted }
        let nextGeneration = base.generation + 1
        let nextID = programID ?? base.id
        let origin = Origin(program: nextID, generation: nextGeneration, steps: [])
        let nextRootJSON = json["root"]
        let nextRoot = try nextRootJSON.flatMap { $0 == .null ? nil : try Element(json: $0, at: origin) }
        let nextErrors = (json["errors"]?.array ?? []).map(RuntimeIssue.init(json:))
        let nextVersion = version
        let nextState = try JSONValue.parse(snapshot)
        // A preview session has no tool provider: it keeps showing the
        // complete session's query status.
        let queries = streaming == true ? base.queries
            : QueryStatus(snapshot: output.querySnapshotJson, evaluationError: output.queryEvaluationError)
        // Publish only after every fallible Swift conversion/read succeeded.
        return Display(root: nextRoot, errors: nextErrors, state: nextState, version: nextVersion,
            sizeLimited: output.sizeLimited, depthLimited: output.depthLimited,
            evaluation: ContinuousClock.now - started, rootJSON: nextRootJSON, handle: output.evaluation,
            id: nextID, generation: nextGeneration, step: step ?? base.step,
            parserError: try output.parserErrorJson.map(JSONValue.parse),
            queryLoading: output.isQueryLoading || (streaming == true && !queries.loading.isEmpty),
            blocked: output.blocked, streaming: streaming ?? base.streaming, queries: queries)
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

    /// The text `failure` shows for an error: its `OpenUIError` message.
    nonisolated static func describe(_ error: any Error) -> String {
        OpenUIError(error).message
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

