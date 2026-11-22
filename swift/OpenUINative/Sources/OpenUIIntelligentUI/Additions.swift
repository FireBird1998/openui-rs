// The one place this library differs from upstream's Intelligent UI as the
// model sees it. Everything else is a copy, and the comparison with
// upstream's recorded JSON Schema and prompt fails if anything else differs.

/// What this library adds to upstream's: Chart's `type` gains `radar` and
/// `radial`, and Chart's description gains the words that explain them.
public enum IntelligentUIAdditions {
    /// Chart types upstream's Chart doesn't have, after its own.
    public static let chartTypes = ["radar", "radial"]

    /// Words added after upstream's Chart description to explain them.
    public static let chartDescription = " radar draws each series as a closed shape over one spoke per x category; radial draws each row as a ring whose length is its first series value relative to the largest row."

    /// One addition as text: what this library writes, and what upstream
    /// writes in its place.
    public struct Change: Sendable, Equatable {
        public let added: String
        public let upstream: String
    }

    /// Every addition as it appears in the JSON Schema, the prompt spec and
    /// the prompt. Putting `upstream` in place of every `added` turns this
    /// library's text into upstream's.
    public static let changes: [Change] = {
        let last = IntelligentUI.upstreamChartTypes.last ?? ""
        let quoted = { (word: String) in "\"\(word)\"" }
        let escaped = { (word: String) in "\\\"\(word)\\\"" }
        return [
            // The type's values in JSON: ..."scatter","radar","radial"]
            Change(added: quoted(last) + chartTypes.map { "," + quoted($0) }.joined() + "]", upstream: quoted(last) + "]"),
            // The type in a signature inside JSON text (the prompt spec's).
            Change(added: escaped(last) + chartTypes.map { " | " + escaped($0) }.joined(), upstream: escaped(last)),
            // The type in a signature: ... | "scatter" | "radar" | "radial"
            Change(added: quoted(last) + chartTypes.map { " | " + quoted($0) }.joined(), upstream: quoted(last)),
            // The description has no character JSON escapes, so it reads the same everywhere.
            Change(added: chartDescription, upstream: ""),
        ]
    }()
}
