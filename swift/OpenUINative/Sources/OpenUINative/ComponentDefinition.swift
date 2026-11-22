// Swift authoring declarations cross the binding only as plain JSON text.
import SwiftUI

/// A prop type in a library definition. Positional props and inline fields
/// keep their declaration order. Optionality and binding belong to a prop.
public struct PropType: Sendable {
    fileprivate let json: DefinitionText

    private init(_ kind: String, _ fields: [(String, DefinitionText)] = []) {
        json = .object([("kind", .string(kind))] + fields)
    }

    public static let string = Self("string")
    public static let number = Self("number")
    public static let boolean = Self("boolean")
    public static let any = Self("any")

    public static func words(_ values: [String]) -> Self {
        Self("enum", [("values", .strings(values))])
    }

    public static func array(_ items: Self) -> Self {
        Self("array", [("items", items.json)])
    }

    public static func union(_ options: [Self]) -> Self {
        Self("union", [("options", .array(options.lazy.map(\.json)))])
    }

    public static func object(_ fields: [FieldDefinition]) -> Self {
        Self("object", [("fields", .array(fields.lazy.map { .object($0.entries) }))])
    }

    public static func record(_ values: Self) -> Self {
        Self("record", [("values", values.json)])
    }

    public static func nullable(_ type: Self) -> Self {
        Self("nullable", [("type", type.json)])
    }

    public static func component(_ name: String) -> Self {
        Self("component", [("name", .string(name))])
    }

    public static func action(_ name: String? = nil) -> Self {
        Self("action", name.map { [("name", .string($0))] } ?? [])
    }

    public static func named(_ name: String, _ type: Self) -> Self {
        Self("named", [("name", .string(name)), ("type", type.json)])
    }
}

/// A field of an inline object.
public struct FieldDefinition: Sendable {
    fileprivate let entries: [(String, DefinitionText)]

    public init(_ name: String, _ type: PropType, optional: Bool = false) {
        var fields = [("name", DefinitionText.string(name)), ("type", type.json)]
        if optional { fields.append(("optional", .trueValue)) }
        entries = fields
    }
}

/// One positional argument of a component. Only a prop can be a binding.
public struct PropDefinition: Sendable {
    fileprivate let json: DefinitionText

    public init(_ name: String, _ type: PropType, optional: Bool = false, binding: Bool = false) {
        var fields = FieldDefinition(name, type, optional: optional).entries
        if binding { fields.append(("binding", .trueValue)) }
        json = .object(fields)
    }
}

public struct ComponentGroup: Sendable {
    fileprivate let json: DefinitionText

    public init(name: String, components: [String], notes: [String]? = nil) {
        var fields: [(String, DefinitionText)] = [("name", .string(name)), ("components", .strings(components))]
        if let notes { fields.append(("notes", .strings(notes))) }
        json = .object(fields)
    }
}

public struct DefinedComponent: Sendable {
    public let name: String
    let draw: ElementDrawing.Draw?
    fileprivate let json: DefinitionText

    fileprivate init(name: String, description: String, props: [PropDefinition], draw: ElementDrawing.Draw? = nil) {
        self.name = name
        self.draw = draw
        json = .object([("name", .string(name)), ("description", .string(description)),
                        ("props", .array(props.lazy.map(\.json)))])
    }
}

/// Declares a component drawn by the fallback.
public func defineComponent(name: String, description: String, props: [PropDefinition]) -> DefinedComponent {
    DefinedComponent(name: name, description: description, props: props)
}

/// Declares a component and the SwiftUI view for its evaluated element.
public func defineComponent<Content: View>(
    name: String, description: String, props: [PropDefinition],
    @ViewBuilder view: @escaping @MainActor @Sendable (Element, _ path: String) -> Content
) -> DefinedComponent {
    DefinedComponent(name: name, description: description, props: props) { element, path in
        AnyView(view(element, path))
    }
}

extension Core {
    /// Builds a component library from Swift declarations on the core thread.
    public func createLibrary(components: [DefinedComponent], groups: [ComponentGroup]? = nil,
                              root: String? = nil) async throws(OpenUIError) -> Library {
        try await run {
            let definition = try libraryDefinition(components: components, groups: groups, root: root)
            return try Library(core: self, definition: definition, components: components, root: root)
        }
    }
}

func libraryDefinition(components: [DefinedComponent], groups: [ComponentGroup]?, root: String?) throws(OpenUIError) -> String {
    var fields: [(String, DefinitionText)] = [("components", .array(components.lazy.map(\.json)))]
    if let groups { fields.append(("componentGroups", .array(groups.lazy.map(\.json)))) }
    if let root { fields.append(("root", .string(root))) }
    guard let text = DefinitionText.object(fields).text else {
        throw OpenUIError(.limitExceeded, "host input byte limit")
    }
    return text
}

// Flat fragments neither encode nor release recursively. Stop joining as
// soon as the default fromDefinition byte limit is exceeded, including when
// a short Swift declaration reuses the same type in both arms of a union.
private struct DefinitionText: Sendable {
    let text: String?
    private static let byteLimit = 8 * 1024 * 1024
    static let trueValue = Self(text: "true")

    static func string(_ value: String) -> Self {
        guard value.utf8.count <= byteLimit else { return Self(text: nil) }
        let quoted = JSONValue.quote(value)
        return Self(text: quoted.utf8.count <= byteLimit ? quoted : nil)
    }

    static func strings(_ values: [String]) -> Self { array(values.lazy.map(string)) }

    static func array(_ items: some Sequence<Self>) -> Self { join("[", items, "]") }

    static func object(_ fields: [(String, Self)]) -> Self {
        // All keys here are wire-format constants. User strings go through string(_:).
        join("{", fields.lazy.map { key, value in
            Self(text: value.text.map { "\"\(key)\":" + $0 })
        }, "}")
    }

    private static func join(_ opening: String, _ items: some Sequence<Self>, _ closing: String) -> Self {
        var out = opening
        for item in items {
            if out.utf8.count > 1 { out += "," }
            guard let text = item.text, text.utf8.count < byteLimit - out.utf8.count else {
                return Self(text: nil)
            }
            out += text
        }
        return Self(text: out + closing)
    }
}
