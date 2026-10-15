import Observation

/// Where an element sits in the tree: the props leading to it from the root,
/// such as `/children/2/header`. The root is `/`. Paths stay put as a stream
/// grows, so a selection survives new chunks.
public enum ElementPath {
    public static let root = "/"

    public static func child(of parent: String, _ key: String) -> String {
        parent == root ? root + key : parent + "/" + key
    }
}

/// The selected element, shared by the drawing and the inspector's tree.
@MainActor @Observable
public final class ElementSelection {
    public private(set) var path: String?
    /// Bumped when the drawing should scroll to the selection (picked in the tree).
    public private(set) var revealInDrawing = 0
    /// Bumped when the tree should scroll to the selection (picked in the drawing).
    public private(set) var revealInTree = 0

    public init() {}

    public enum Origin { case drawing, tree }

    public func select(_ path: String?, from origin: Origin) {
        self.path = path
        switch origin {
        case .drawing: revealInTree += 1
        case .tree: revealInDrawing += 1
        }
    }

    public func clear() { path = nil }
}

/// One row of the inspector's element tree.
public struct TreeRow: Identifiable, Sendable, Equatable {
    /// The element's path.
    public var id: String
    public var depth: Int
    public var typeName: String
    public var statementId: String?
    public var partial: Bool
    /// The prop it sits in, for elements below the root.
    public var key: String?

    /// Every element under `root`, depth first, in the order the drawing shows them.
    public static func rows(of root: Element) -> [TreeRow] {
        var rows: [TreeRow] = []
        func visit(_ element: Element, path: String, depth: Int, key: String?) {
            rows.append(TreeRow(
                id: path, depth: depth, typeName: element.typeName,
                statementId: element.statementId, partial: element.partial, key: key
            ))
            for prop in element.props {
                walk(prop.value, path: ElementPath.child(of: path, prop.name), depth: depth + 1, key: prop.name)
            }
        }
        func walk(_ value: PropValue, path: String, depth: Int, key: String) {
            switch value {
            case let .element(element):
                visit(element, path: path, depth: depth, key: key)
            case let .list(items):
                for (i, item) in items.enumerated() {
                    walk(item, path: ElementPath.child(of: path, String(i)), depth: depth, key: "\(key)[\(i)]")
                }
            case let .record(fields):
                for field in fields {
                    walk(field.value, path: ElementPath.child(of: path, field.name), depth: depth, key: "\(key).\(field.name)")
                }
            case .expression, .scalar:
                break
            }
        }
        visit(root, path: ElementPath.root, depth: 0, key: nil)
        return rows
    }
}
