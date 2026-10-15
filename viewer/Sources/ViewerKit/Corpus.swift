import Foundation

/// One corpus case: a program, the component library it's parsed against,
/// and an optional root name.
public struct CorpusCase: Sendable, Hashable, Identifiable {
    public var name: String
    /// Where the case came from: hand-written, docs, fuzz…
    public var source: String
    public var schemaName: String
    public var file: String
    public var root: String?
    public var id: String { name }
}

/// The conformance corpus, read from the manifests the core's tests use.
public struct Corpus: Sendable {
    public var directory: URL
    public var cases: [CorpusCase]
    /// Schema name to file, relative to `directory`.
    public var schemas: [String: String]

    /// The conformance folder of the repository this viewer was built from.
    public static var defaultDirectory: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // ViewerKit
            .deletingLastPathComponent() // Sources
            .deletingLastPathComponent() // viewer
            .deletingLastPathComponent() // the repository
            .appendingPathComponent("conformance")
    }

    public static let manifests = ["manifest.json", "imported.json", "fuzz.json", "synthetic.json"]

    public static func load(from directory: URL = defaultDirectory) throws -> Corpus {
        var corpus = Corpus(directory: directory, cases: [], schemas: [:])
        for file in manifests {
            let url = directory.appendingPathComponent(file)
            guard FileManager.default.fileExists(atPath: url.path) else { continue }
            let manifest = try JSONValue.parse(String(contentsOf: url, encoding: .utf8))
            if case let .object(schemas)? = manifest["schemas"] {
                for (name, path) in schemas {
                    if let path = path.string { corpus.schemas[name] = path }
                }
            }
            for c in manifest["cases"]?.array ?? [] {
                guard let name = c["name"]?.string, let schema = c["schema"]?.string,
                      let file = c["file"]?.string
                else { continue }
                corpus.cases.append(CorpusCase(
                    name: name,
                    source: c["source"]?.string ?? "",
                    schemaName: schema,
                    file: file,
                    root: c["root"]?.string
                ))
            }
        }
        if corpus.cases.isEmpty {
            throw DecodeError(description: "no corpus cases in \(directory.path)")
        }
        return corpus
    }

    public func program(of c: CorpusCase) throws -> String {
        try String(contentsOf: directory.appendingPathComponent(c.file), encoding: .utf8)
    }

    public func schema(of c: CorpusCase) throws -> String {
        guard let path = schemas[c.schemaName] else {
            throw DecodeError(description: "\(c.name) uses unknown schema \(c.schemaName)")
        }
        return try String(contentsOf: directory.appendingPathComponent(path), encoding: .utf8)
    }
}
