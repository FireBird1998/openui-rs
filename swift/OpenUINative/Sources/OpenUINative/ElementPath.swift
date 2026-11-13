/// Where an element sits in the tree: the props leading to it from the root,
/// such as `/children/2/header`. The root is `/`. Paths stay put as a stream
/// grows, so a selection survives new chunks.
public enum ElementPath {
    public static let root = "/"

    public static func child(of parent: String, _ key: String) -> String {
        parent == root ? root + key : parent + "/" + key
    }

    /// The paths above `path`, outermost first, root excluded:
    /// `/children/2/header` has `/children` and `/children/2`.
    public static func ancestors(of path: String) -> [String] {
        let parts = path.split(separator: "/")
        return parts.indices.dropLast().map { "/" + parts[...$0].joined(separator: "/") }
    }
}
