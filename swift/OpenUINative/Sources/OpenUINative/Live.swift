import Foundation
import Observation
import OpenUIFFI

/// A program kept live by a runtime session: its parse result evaluated
/// against a store of state, redrawn whenever the store changes. Draw it with
/// `ElementView(live:)`; built-in buttons run their actions, built-in inputs
/// and selects read and write through binding markers or form fields, and
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
    public private(set) var root: Element?
    /// What failed while evaluating the tree, in the runtime's order.
    public private(set) var errors: [RuntimeIssue] = []
    /// The store's snapshot: every state variable and form field that has a
    /// value, by name, in the store's order.
    public private(set) var state: JSONValue = .object([])
    /// The store's version: it rises on every real change.
    public private(set) var version: UInt64 = 0
    /// The last evaluation was cut at the runtime's size or work limit.
    public private(set) var sizeLimited = false
    /// The last evaluation was cut at the runtime's depth limit.
    public private(set) var depthLimited = false
    /// Why the last interaction from a built-in view did nothing or stopped
    /// part way, such as a write the store refused; nil once one succeeds.
    public private(set) var failure: String?
    /// How long the last evaluation took, decoding included.
    public private(set) var evaluation: Duration?
    /// A response is still streaming into the program. The app sets it while
    /// the program's text is arriving and clears it once the response is
    /// complete. Meanwhile built-in buttons, inputs and selects are disabled,
    /// a click that still arrives is ignored, and `run` refuses, as upstream's
    /// renderer does until its response is complete. Writes, `set` and
    /// `resume` still work.
    public var isStreaming = false
    /// Receives every host event, in order: what the program asks the app to
    /// do (send a message to the assistant, open a URL, a custom action, a
    /// query or mutation to run). It's called after the store has changed and
    /// the tree has been evaluated again.
    @ObservationIgnored public var onHostEvent: @MainActor (HostEvent) -> Void

    /// Internal so tests can reach it: the viewer's count its pending
    /// mutations, and the package's dispose it under a redraw.
    @ObservationIgnored let session: RuntimeSession
    /// The pristine state variables at a store version, read once.
    @ObservationIgnored private var pristineCache: (version: UInt64, names: Set<String>)?
    @ObservationIgnored private var handle: EvaluationHandle?
    @ObservationIgnored private var hasProgram = false
    /// The last evaluated tree's root as the runtime gave it.
    @ObservationIgnored private(set) var rootJSON: JSONValue?
    /// This live program's identity, carried by its tree's elements and markers.
    @ObservationIgnored private let id: UInt64
    /// Which evaluation `root` is; it rises with each one.
    @ObservationIgnored private var generation: UInt64 = 0
    private static var nextID: UInt64 = 0

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
        Self.nextID += 1
        id = Self.nextID
        session = try RuntimeSession(library: library.inner, promptSpec: promptSpec)
        for binding in bindings {
            try session.markBinding(component: binding.component, prop: binding.prop)
        }
        self.onHostEvent = onHostEvent
    }

    /// Shows a parse result: the store takes its state declarations (a value
    /// set before survives; a declared starting value fills in or replaces
    /// one still pristine), `persisted` values win over both, and the tree is
    /// evaluated. Call it with each new step of a stream.
    public func show(_ step: Step, persisted: JSONValue? = nil) throws {
        pristineCache = nil
        try session.setProgram(program: step.parsed)
        hasProgram = true
        let initialized = try session.initialize(persisted: persisted?.compact)
        try evaluate()
        failure = initialized.refused.isEmpty ? nil
            : initialized.refused.map { "\($0.name): \(Self.describe($0.error))" }.joined(separator: "\n")
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
        let names = Set(entries.map(\.key).filter { (try? session.isPristine(name: $0)) ?? false })
        pristineCache = (version, names)
        return names
    }

    /// Sets a state variable (nil makes it undefined) and redraws if that
    /// changed anything. Throws if the store refuses the value.
    public func set(_ name: String, to value: JSONValue?) throws {
        _ = try session.set(name: name, json: value?.compact)
        try refresh()
    }

    /// Writes a value through a binding marker from the tree: the marker's
    /// expression turns it into what's stored. A marker from this program's
    /// current evaluation writes. One from an earlier evaluation (a second
    /// keystroke that arrived before the redraw) writes only while the same
    /// marker is at its place in the tree now; otherwise it throws and
    /// changes nothing.
    public func write(_ marker: BindingMarker, _ value: JSONValue) throws {
        guard let origin = marker.origin, origin.program == id else { throw Self.stale }
        if origin.generation != generation {
            guard case let .binding(now)? = locate(origin.steps)?.value, now.json == marker.json else { throw Self.stale }
        }
        _ = try session.writeBinding(marker: marker.json.compact, json: value.compact)
        try refresh()
    }

    /// An unbound form field's value: in `form`'s fields, or a top-level
    /// field when `form` is nil; nil when it has none. It's read from `state`,
    /// with no call to the session, by the runtime's rules: a field's
    /// `{value, componentType}` unwraps to its value, and one written as
    /// undefined, which keeps only its component type, has none.
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
        if entries.allSatisfy({ $0.key == "componentType" && $0.value.string != nil }) { return nil }
        return raw
    }

    /// Writes an unbound form field and redraws if that changed anything.
    /// `componentType` is kept beside the value and travels in events' form
    /// state; upstream's own Input and Select give none, while a component
    /// that writes its field itself (such as Chips) gives its name.
    public func setField(form: String?, componentType: String? = nil, name: String, to value: JSONValue?) throws {
        _ = try session.setField(form: form, componentType: componentType, name: name, json: value?.compact, save: true)
        try refresh()
    }

    /// Writes what a person typed or picked in a built-in input or select
    /// drawn from `element`: through its binding marker, or else into its
    /// form field. Refused, changing nothing, when what was drawn is gone
    /// (see `current(_:form:)`).
    func enter(_ value: JSONValue, into element: Element, form: String?) throws {
        guard let element = current(element, form: form) else { throw Self.stale }
        let props = PropReader(element: element)
        if case let .binding(marker)? = props.value("value") {
            try write(marker, value)
        } else {
            // Upstream's Input and Select write their field with no component
            // type (react-lang's useStateField passes undefined); so do these.
            try setField(form: form, name: try props.text("name").plain, to: value)
        }
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
        do { try refresh() } catch { redrawError = error }
        let stepFailure = output.failure.map { failure in
            (failure.step.map { "Step \($0): " } ?? "") + Self.describe(failure.error)
        }
        failure = [stepFailure, redrawError.map(Self.describe)].compactMap { $0 }.joined(separator: "\n")
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
    private func refresh() throws {
        pristineCache = nil
        if try session.version() != version {
            if hasProgram { try evaluate() } else { try readStore() }
        }
    }

    private func evaluate() throws {
        let started = ContinuousClock.now
        let output = try session.evaluate()
        let json = try JSONValue.parse(output.json)
        generation += 1
        let origin = Origin(program: id, generation: generation, steps: [])
        rootJSON = json["root"]
        root = try json["root"].flatMap { $0 == .null ? nil : try Element(json: $0, at: origin) }
        errors = (json["errors"]?.array ?? []).map(RuntimeIssue.init(json:))
        handle = output.evaluation
        sizeLimited = output.sizeLimited
        depthLimited = output.depthLimited
        try readStore()
        evaluation = ContinuousClock.now - started
    }

    /// Reads the store's version and snapshot. The version is kept only once
    /// the snapshot has been read, so a failed read is retried on the next
    /// refresh rather than taken as up to date.
    private func readStore() throws {
        let version = try session.version()
        state = try JSONValue.parse(session.snapshot())
        self.version = version
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
