import OpenUIFFI

/// A component library loaded into the core, shared by every parse against it.
public struct Library: Sendable {
    let inner: ComponentLibrary

    /// Loads the library from its JSON Schema; throws if the schema isn't JSON.
    public init(schema: String) throws {
        inner = try ComponentLibrary(schema: schema)
    }

    public var componentNames: [String] { inner.componentNames() }
}

/// Parses a whole program one-shot through the core's binding.
public func parseOneShot(_ program: String, library: Library, root: String? = nil) throws -> ParseResult {
    let output = OpenUIFFI.parse(program: program, library: library.inner, rootName: root)
    return try ParseResult(json: output.json, depthLimited: output.depthLimited, sizeLimited: output.sizeLimited)
}
