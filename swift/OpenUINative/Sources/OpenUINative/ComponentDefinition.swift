// Swift authoring declarations cross the binding only as plain JSON text.
import OpenUIFFI
import SwiftUI

/// RuntimeLimits.maxJsonBytes for the default sessions created by OpenUINative.
public let defaultHostByteLimit = 8 * 1024 * 1024

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

/// A field of an inline object, or a parameter of a function or action.
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
    var formName: (@MainActor @Sendable (Element) -> String?)?
    fileprivate let json: DefinitionText

    fileprivate init(name: String, description: String, props: [PropDefinition], draw: ElementDrawing.Draw? = nil) {
        self.name = name
        self.draw = draw
        json = .object([("name", .string(name)), ("description", .string(description)),
                        ("props", .array(props.lazy.map(\.json)))])
    }

    /// Resolves the field scope of a declared Form during stale interaction checks.
    package func withFormName(_ name: @escaping @MainActor @Sendable (Element) -> String?) -> Self {
        var definition = self
        definition.formName = name
        return definition
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

public struct DefinedFunction: Sendable {
    public let name: String
    let callback: (any LibraryFunction)?
    fileprivate let json: DefinitionText
}

/// The synchronous implementation receives the checked, named arguments on
/// the session's callback thread. Omitting it makes calls evaluate to null.
public func defineFunction(name: String, description: String, params: [FieldDefinition],
                           returns: PropType? = nil,
                           implementation: (@Sendable (JSONValue) throws -> JSONValue)? = nil) -> DefinedFunction {
    var fields = callFields(name: name, description: description, params: params)
    if let returns { fields.append(("returns", returns.json)) }
    return DefinedFunction(name: name, callback: implementation.map(FunctionCallback.init), json: .object(fields))
}

private final class FunctionCallback: LibraryFunction {
    let implementation: @Sendable (JSONValue) throws -> JSONValue

    init(_ implementation: @escaping @Sendable (JSONValue) throws -> JSONValue) {
        self.implementation = implementation
    }

    func call(argumentsJson: String) throws -> String? {
        do {
            let arguments = try JSONValue.parse(argumentsJson)
            let result = try implementation(arguments)
            guard let json = result.compact(byteLimit: defaultHostByteLimit) else {
                throw OpenUIError(.limitExceeded, "host function result byte limit")
            }
            return json
        } catch {
            throw FunctionError.Thrown(reason: String(describing: error))
        }
    }
}

public struct DefinedAction: Sendable {
    public let name: String
    fileprivate let json: DefinitionText
}

/// Declares a custom action delivered through the live program's host event handler.
public func defineAction(name: String, description: String, params: [FieldDefinition]) -> DefinedAction {
    DefinedAction(name: name, json: .object(callFields(name: name, description: description, params: params)))
}

private func callFields(name: String, description: String, params: [FieldDefinition]) -> [(String, DefinitionText)] {
    [("name", .string(name)), ("description", .string(description)),
     ("params", .array(params.lazy.map { .object($0.entries) }))]
}

extension Core {
    /// Builds a component library from Swift declarations on the core thread.
    public func createLibrary(
        components: [DefinedComponent], groups: [ComponentGroup]? = nil,
        functions: [DefinedFunction] = [], actions: [DefinedAction] = [], root: String? = nil
    ) async throws(OpenUIError) -> Library {
        try await run {
            let definition = try libraryDefinition(components: components, groups: groups, functions: functions,
                                                   actions: actions, root: root)
            return try Library(core: self, definition: definition, components: components, functions: functions, root: root)
        }
    }
}

func libraryDefinition(components: [DefinedComponent], groups: [ComponentGroup]?,
                       functions: [DefinedFunction] = [], actions: [DefinedAction] = [], root: String?) throws(OpenUIError) -> String {
    var fields: [(String, DefinitionText)] = [("components", .array(components.lazy.map(\.json)))]
    if let groups { fields.append(("componentGroups", .array(groups.lazy.map(\.json)))) }
    if !functions.isEmpty { fields.append(("functions", .array(functions.lazy.map(\.json)))) }
    if !actions.isEmpty { fields.append(("actions", .array(actions.lazy.map(\.json)))) }
    if let root { fields.append(("root", .string(root))) }
    guard let text = DefinitionText.object(fields).text else {
        throw .hostInputByteLimitExceeded
    }
    return text
}

// Flat fragments neither encode nor release recursively. Stop joining as
// soon as the default fromDefinition byte limit is exceeded, including when
// a short Swift declaration reuses the same type in both arms of a union.
private struct DefinitionText: Sendable {
    let text: String?
    static let trueValue = Self(text: "true")

    static func string(_ value: String) -> Self {
        guard value.utf8.count <= defaultHostByteLimit else { return Self(text: nil) }
        let quoted = JSONValue.quote(value)
        return Self(text: quoted.utf8.count <= defaultHostByteLimit ? quoted : nil)
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
            guard let text = item.text, text.utf8.count < defaultHostByteLimit - out.utf8.count else {
                return Self(text: nil)
            }
            out += text
        }
        return Self(text: out + closing)
    }
}
