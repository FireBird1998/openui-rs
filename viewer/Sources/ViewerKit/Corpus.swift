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

/// Corpus cases from one source, for the sidebar.
public struct CorpusSection: Identifiable, Sendable {
    public var id: String { source }
    public var source: String
    public var cases: [CorpusCase]

    public var title: String {
        switch source {
        case "hand-written": "Hand-written"
        case "docs": "Upstream docs"
        case "examples": "Upstream examples"
        case "reference-test": "Upstream tests"
        case "benchmark": "Upstream benchmarks"
        case "fuzz": "Found by fuzzing"
        case "synthetic": "Synthetic"
        default: source.isEmpty ? "Other" : source
        }
    }
}

extension Corpus {
    /// The cases whose names contain `search` (all when it's empty), grouped
    /// by source in the manifests' order.
    public func sections(matching search: String) -> [CorpusSection] {
        var sections: [CorpusSection] = []
        for c in cases where search.isEmpty || c.name.localizedCaseInsensitiveContains(search) {
            if let i = sections.firstIndex(where: { $0.source == c.source }) {
                sections[i].cases.append(c)
            } else {
                sections.append(CorpusSection(source: c.source, cases: [c]))
            }
        }
        return sections
    }

    /// The prompt spec for a component library (`prompts/<name>-schema.json`),
    /// which the system prompt is made from, if the corpus has one.
    public func promptSpec(forLibrary name: String) -> String? {
        try? String(contentsOf: directory.appendingPathComponent("prompts/\(name)-schema.json"), encoding: .utf8)
    }

    /// The prompt spec for a library with edit mode on (`promptOptions.editMode`),
    /// keeping its other options (examples, rules), for follow-up requests that
    /// answer with a patch.
    public func promptSpec(forLibrary name: String, editMode: Bool) -> String? {
        guard let spec = promptSpec(forLibrary: name) else { return nil }
        guard editMode, case var .object(entries) = try? JSONValue.parse(spec) else { return spec }
        var options: [(key: String, value: JSONValue)] = []
        if case let .object(existing)? = entries.first(where: { $0.key == "promptOptions" })?.value {
            options = existing.filter { $0.key != "editMode" }
        }
        options.append((key: "editMode", value: .bool(true)))
        entries.removeAll { $0.key == "promptOptions" }
        entries.append((key: "promptOptions", value: .object(options)))
        return JSONValue.object(entries).compact
    }

    /// The library live requests start with: upstream's own react-ui library,
    /// the one its apps ask models for.
    public static let preferredLibrary = "react-ui"

    /// The libraries live requests can use: those with a schema and a prompt spec.
    public var promptLibraries: [String] {
        schemas.keys.sorted().filter {
            FileManager.default.fileExists(atPath: directory.appendingPathComponent("prompts/\($0)-schema.json").path)
        }
    }

    /// The reference's step fingerprints for a case, if the corpus has them.
    public func golden(for c: CorpusCase) -> GoldenSteps? {
        try? GoldenSteps.load(caseName: c.name, in: directory)
    }
}
