/// Reads an element's props for a native view. A prop the view needs to draw
/// must be there and of the right type, or the element is drawn as a labeled
/// box instead. While the element is still arriving, a missing prop reads as
/// empty, so the element doesn't flip between box and native view mid-stream.
/// Style props with an unknown value fall back to their default.
struct PropReader {
    let element: Element

    struct Unusable: Error {}

    private var lenient: Bool { element.partial }

    /// The prop's value, nil when it's absent or null.
    func value(_ name: String) -> PropValue? {
        guard let value = element.props.first(where: { $0.name == name })?.value else { return nil }
        if case .scalar(.null) = value { return nil }
        return value
    }

    // MARK: Text

    func text(_ name: String) throws(Unusable) -> TextPiece {
        guard let piece = try optionalText(name) else {
            if lenient { return .literal("") }
            throw Unusable()
        }
        return piece
    }

    func optionalText(_ name: String) throws(Unusable) -> TextPiece? {
        guard let value = value(name) else { return nil }
        switch value {
        case let .scalar(.string(s)): return .literal(s)
        case let .scalar(.number(n)): return .literal(JSONValue.format(n))
        case let .scalar(.bool(b)): return .literal(b ? "true" : "false")
        case .expression: return .dynamic(value.summary)
        default: throw Unusable()
        }
    }

    /// One of a fixed set of words, or `fallback` when absent or unknown.
    func keyword(_ name: String, _ fallback: String) -> String {
        if case let .scalar(.string(s))? = value(name) { s } else { fallback }
    }

    func bool(_ name: String) -> Bool {
        if case let .scalar(.bool(b))? = value(name) { b } else { false }
    }

    // MARK: Numbers

    func number(_ name: String) throws(Unusable) -> Double {
        switch value(name) {
        case let .scalar(.number(n))?: return n
        case nil where lenient: return 0
        default: throw Unusable()
        }
    }

    func numbers(_ name: String) throws(Unusable) -> [Double] {
        try list(name).map { item throws(Unusable) in
            guard case let .scalar(.number(n)) = item else { throw Unusable() }
            return n
        }
    }

    func strings(_ name: String) throws(Unusable) -> [String] {
        try list(name).map { item throws(Unusable) in
            switch item {
            case let .scalar(.string(s)): return s
            case let .scalar(.number(n)): return JSONValue.format(n)
            default: throw Unusable()
            }
        }
    }

    // MARK: Lists and elements

    /// A list prop; a dynamic list (an expression) can't be drawn.
    func list(_ name: String) throws(Unusable) -> [PropValue] {
        guard let items = try optionalList(name) else {
            if lenient { return [] }
            throw Unusable()
        }
        return items
    }

    func optionalList(_ name: String) throws(Unusable) -> [PropValue]? {
        guard let value = value(name) else { return nil }
        guard case let .list(items) = value else { throw Unusable() }
        return items
    }

    /// The list's items, each an element of type `typeName`, with its index
    /// (for its path). Nulls are skipped.
    func elements(_ name: String, of typeName: String) throws(Unusable) -> [(index: Int, element: Element)] {
        var found: [(index: Int, element: Element)] = []
        for (i, item) in try list(name).enumerated() {
            switch item {
            case let .element(e) where e.typeName == typeName: found.append((i, e))
            case .scalar(.null): continue
            default: throw Unusable()
            }
        }
        return found
    }

    func optionalElement(_ name: String) throws(Unusable) -> Element? {
        guard let value = value(name) else { return nil }
        guard case let .element(e) = value else { throw Unusable() }
        return e
    }

    func element(_ name: String) throws(Unusable) -> Element? {
        guard let e = try optionalElement(name) else {
            if lenient { return nil }
            throw Unusable()
        }
        return e
    }
}

/// Text a native view shows: literal, or an expression only a runtime could
/// evaluate, shown as its source.
enum TextPiece: Equatable {
    case literal(String)
    case dynamic(String)
}
