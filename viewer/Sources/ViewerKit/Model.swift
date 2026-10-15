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
}

public struct Prop: Sendable {
    public var name: String
    public var value: PropValue
}

/// A prop's value, sorted into what the viewer draws differently.
public indirect enum PropValue: Sendable {
    case element(Element)
    case list([PropValue])
    /// A plain object; its fields may hold elements too.
    case record([Prop])
    /// An expression the core leaves for a runtime to evaluate.
    case expression(JSONValue)
    /// A string, number, boolean or null.
    case scalar(JSONValue)
}

public struct DecodeError: Error, CustomStringConvertible {
    public let description: String
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

extension Element {
    /// Whether a JSON value is an element node.
    static func isElement(_ json: JSONValue) -> Bool {
        json["type"]?.string == "element" && json["typeName"]?.string != nil
    }

    init(json: JSONValue) throws {
        guard Element.isElement(json), let typeName = json["typeName"]?.string else {
            throw DecodeError(description: "not an element: \(json.compact.prefix(80))")
        }
        guard case let .object(props)? = json["props"] else {
            throw DecodeError(description: "\(typeName) has no props object")
        }
        self.typeName = typeName
        self.props = props.map { Prop(name: $0.key, value: PropValue(json: $0.value)) }
        self.partial = json["partial"]?.bool ?? false
        self.hasDynamicProps = json["hasDynamicProps"]?.bool ?? false
        self.statementId = json["statementId"]?.string
    }
}

extension PropValue {
    init(json: JSONValue) {
        switch json {
        case let .array(items):
            self = .list(items.map(PropValue.init(json:)))
        case let .object(entries):
            if Element.isElement(json), let element = try? Element(json: json) {
                self = .element(element)
            } else if let kind = json["k"]?.string, expressionKinds.contains(kind) {
                self = .expression(json)
            } else {
                self = .record(entries.map { Prop(name: $0.key, value: PropValue(json: $0.value)) })
            }
        default:
            self = .scalar(json)
        }
    }
}
