/// The viewer's copy of a parse result, decoded from the JSON the core
/// produces (the reference's shape, byte for byte).
public struct ParseResult: Sendable {
    public var root: Element?
    public var meta: Meta
    /// Nesting went past the core's depth limit and was cut.
    public var depthLimited: Bool
    /// The result grew past the core's size limit and was cut.
    public var sizeLimited: Bool
    /// The whole result as decoded, for the parts not modelled yet.
    public var json: JSONValue
}

public struct Meta: Sendable {
    public var incomplete: Bool
    public var unresolved: [String]
    public var orphaned: [String]
    public var statementCount: Int
    public var errors: [ValidationIssue]
}

public struct ValidationIssue: Sendable {
    public var code: String
    public var component: String
    public var path: String
    public var message: String
    public var statementId: String?
}

/// One component instance in the tree.
public struct Element: Sendable {
    public var typeName: String
    public var props: [Prop]
    /// Still arriving: the statement or one of its parts isn't complete yet.
    public var partial: Bool
    public var hasDynamicProps: Bool
    public var statementId: String?
    /// For an element of a live program's evaluated tree: which program,
    /// which evaluation, and where in the tree. What a click or a write from
    /// it checks before it acts.
    var origin: Origin? = nil

    /// Where the element sits in the live program's evaluated tree it came
    /// from; nil for a parse result's elements. `ElementView` draws an element
    /// at this path whatever path it's given.
    public var path: String? { origin?.path }
}

/// Where an element or marker of an evaluated tree came from.
struct Origin: Sendable, Equatable {
    /// The live program's identity.
    var program: UInt64
    /// Which of its evaluations.
    var generation: UInt64
    /// The prop names, list indices and record keys leading to it from the
    /// root. Kept apart, since a record key may hold a `/`.
    var steps: [String]

    var path: String { steps.reduce(ElementPath.root) { ElementPath.child(of: $0, $1) } }
}

public struct Prop: Sendable {
    public var name: String
    public var value: PropValue
}

/// A prop's value, sorted into what a drawing handles differently.
public indirect enum PropValue: Sendable {
    case element(Element)
    case list([PropValue])
    /// A plain object; its fields may hold elements too.
    case record([Prop])
    /// An expression not evaluated: in a parse result, every expression; in
    /// an evaluated tree, only a prop whose evaluation failed or was cut.
    case expression(JSONValue)
    /// In an evaluated tree, a binding prop given a state variable: where an
    /// input reads and writes.
    case binding(BindingMarker)
    /// In an evaluated tree, an action: the plan a click runs.
    case plan(ActionPlan)
    /// A string, number, boolean or null.
    case scalar(JSONValue)
}

/// A binding marker: the state variable a binding prop reads, and the
/// expression a written value goes through before it's stored there.
/// Write through it with `LiveProgram.write(_:_:)`; a marker from an
/// evaluation that has since been replaced writes nothing.
public struct BindingMarker: Sendable, Equatable {
    /// The state variable, with its `$`.
    public var target: String
    /// The marker as the runtime gave it.
    public var json: JSONValue
    /// The live program and evaluation it came from.
    var origin: Origin? = nil
}

/// An action plan: the steps a click runs, in order. Run it with
/// `LiveProgram.run(_:of:message:form:)`, which checks the element is still
/// the one in the tree. Drawing one grants nothing: only the session's own
/// plan at that place in the current evaluation runs.
public struct ActionPlan: Sendable, Equatable {
    public var steps: [JSONValue]
}

public struct DecodeError: Error, CustomStringConvertible {
    public let description: String

    public init(description: String) {
        self.description = description
    }
}

/// The `k` values of expression nodes in the core's JSON.
private let expressionKinds: Set<String> = [
    "Arr", "Assign", "BinOp", "Bool", "Comp", "Index", "Member", "Null", "Num", "Obj",
    "Ph", "Ref", "RuntimeRef", "StateRef", "Str", "Ternary", "UnaryOp",
]

extension ParseResult {
    public init(json text: String, depthLimited: Bool, sizeLimited: Bool) throws {
        let json = try JSONValue.parse(text)
        guard let meta = json["meta"] else { throw DecodeError(description: "no meta") }
        self.root = try json["root"].flatMap { $0 == .null ? nil : try Element(json: $0) }
        self.meta = Meta(
            incomplete: meta["incomplete"]?.bool ?? false,
            unresolved: meta["unresolved"]?.array?.compactMap(\.string) ?? [],
            orphaned: meta["orphaned"]?.array?.compactMap(\.string) ?? [],
            statementCount: Int(meta["statementCount"]?.number ?? 0),
            errors: (meta["errors"]?.array ?? []).map { e in
                ValidationIssue(
                    code: e["code"]?.string ?? "",
                    component: e["component"]?.string ?? "",
                    path: e["path"]?.string ?? "",
                    message: e["message"]?.string ?? "",
                    statementId: e["statementId"]?.string
                )
            }
        )
        self.depthLimited = depthLimited
        self.sizeLimited = sizeLimited
        self.json = json
    }
}

/// Decoding is lossless: every JSON value decodes to a `PropValue` whose
/// `json` is that value again. An element, a binding marker and an action
/// plan are recognised only in exactly the shape the core and the runtime
/// write them; anything else (data with a `steps` key, say) is a record.
extension Element {
    /// The keys of an element node, in the order the core writes them;
    /// `statementId` may follow.
    private static let keys = ["type", "typeName", "props", "partial", "hasDynamicProps"]

    /// Whether a JSON value is an element node, in exactly the core's shape.
    static func isElement(_ json: JSONValue) -> Bool {
        guard case let .object(entries) = json, entries.count == 5 || entries.count == 6,
              zip(entries, keys).allSatisfy({ $0.key == $1 }),
              entries[0].value == .string("element"), entries[1].value.string != nil,
              case .object = entries[2].value, entries[3].value.bool != nil, entries[4].value.bool != nil
        else { return false }
        return entries.count == 5 || (entries[5].key == "statementId" && entries[5].value.string != nil)
    }

    init(json: JSONValue) throws {
        try self.init(json: json, at: nil)
    }

    /// With `origin`, the element of a live program's evaluated tree at that
    /// path, and every element inside it at its own path.
    init(json: JSONValue, at origin: Origin?) throws {
        guard Element.isElement(json), let typeName = json["typeName"]?.string,
              case let .object(props)? = json["props"]
        else {
            throw DecodeError(description: "not an element: \(json.compact.prefix(80))")
        }
        self.typeName = typeName
        self.props = props.map { prop in
            Prop(name: prop.key, value: PropValue(json: prop.value, at: origin.map { $0.child(prop.key) }))
        }
        self.partial = json["partial"]?.bool ?? false
        self.hasDynamicProps = json["hasDynamicProps"]?.bool ?? false
        self.statementId = json["statementId"]?.string
        self.origin = origin
    }
}

extension Origin {
    func child(_ key: String) -> Origin {
        Origin(program: program, generation: generation, steps: steps + [key])
    }
}

extension PropValue {
    init(json: JSONValue) {
        self.init(json: json, at: nil)
    }

    init(json: JSONValue, at origin: Origin?) {
        switch json {
        case let .array(items):
            self = .list(items.enumerated().map { i, item in
                PropValue(json: item, at: origin.map { $0.child(String(i)) })
            })
        case let .object(entries):
            if Element.isElement(json), let element = try? Element(json: json, at: origin) {
                self = .element(element)
            } else if let target = Self.markerTarget(entries) {
                self = .binding(BindingMarker(target: target, json: json, origin: origin))
            } else if let steps = Self.planSteps(entries) {
                self = .plan(ActionPlan(steps: steps))
            } else if let kind = json["k"]?.string, expressionKinds.contains(kind) {
                self = .expression(json)
            } else {
                self = .record(entries.map { entry in
                    Prop(name: entry.key, value: PropValue(json: entry.value, at: origin.map { $0.child(entry.key) }))
                })
            }
        default:
            self = .scalar(json)
        }
    }

    /// A binding marker is exactly `{__reactive: "assign", target, expr}`.
    private static func markerTarget(_ entries: [(key: String, value: JSONValue)]) -> String? {
        guard entries.map(\.key) == ["__reactive", "target", "expr"],
              entries[0].value == .string("assign") else { return nil }
        return entries[1].value.string
    }

    /// A plan is exactly `{steps: [...]}`, every step an object whose first
    /// key is its string `type`.
    private static func planSteps(_ entries: [(key: String, value: JSONValue)]) -> [JSONValue]? {
        guard entries.count == 1, entries[0].key == "steps", case let .array(steps) = entries[0].value,
              steps.allSatisfy({ step in
                  if case let .object(fields) = step, fields.first?.key == "type", fields.first?.value.string != nil {
                      return true
                  }
                  return false
              })
        else { return nil }
        return steps
    }
}

extension PropValue {
    /// Whether an element sits anywhere inside this value.
    public var holdsElements: Bool {
        switch self {
        case .element: true
        case let .list(items): items.contains { $0.holdsElements }
        case let .record(fields): fields.contains { $0.value.holdsElements }
        case .expression, .binding, .plan, .scalar: false
        }
    }

    /// One line of text for a value with no elements in it.
    public var summary: String {
        switch self {
        case let .scalar(json): json.compact
        case let .expression(json): "ƒ " + json.compact
        case let .binding(marker): "⇄ " + marker.target
        case let .plan(plan): "▶ action, \(plan.steps.count) step" + (plan.steps.count == 1 ? "" : "s")
        case let .list(items): "[" + items.map(\.summary).joined(separator: ", ") + "]"
        case let .record(fields): "{" + fields.map { $0.name + ": " + $0.value.summary }.joined(separator: ", ") + "}"
        case let .element(element): element.typeName + "(…)"
        }
    }
}

extension Element {
    /// The element as JSON, in the core's shape.
    var json: JSONValue {
        var entries: [(key: String, value: JSONValue)] = [
            ("type", .string("element")),
            ("typeName", .string(typeName)),
            ("props", .object(props.map { ($0.name, $0.value.json) })),
            ("partial", .bool(partial)),
            ("hasDynamicProps", .bool(hasDynamicProps)),
        ]
        if let statementId { entries.append(("statementId", .string(statementId))) }
        return .object(entries)
    }
}

extension PropValue {
    /// The value as JSON again.
    var json: JSONValue {
        switch self {
        case let .element(element): element.json
        case let .list(items): .array(items.map(\.json))
        case let .record(fields): .object(fields.map { ($0.name, $0.value.json) })
        case let .binding(marker): marker.json
        case let .plan(plan): .object([("steps", .array(plan.steps))])
        case let .expression(json), let .scalar(json): json
        }
    }
}
