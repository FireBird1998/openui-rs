import Foundation

/// The pinned library's flat block dialect. A line belongs to exactly one
/// block, except for the single lookahead used to recognize a table.
enum MarkdownBlock: Equatable {
    case heading(Int, String)
    case paragraph(String)
    case list(start: Int?, items: [String])
    case quote(String)
    case code(String)
    case rule
    case table(head: [String], rows: [[String]])

    static func parse(_ source: String) -> [Self] {
        // Split on scalars so punctuation followed by a combining mark
        // still counts as punctuation, and handle CRLF as one newline.
        let normalized = source.replacingOccurrences(of: "\r\n", with: "\n", options: .literal)
            .replacingOccurrences(of: "\r", with: "\n", options: .literal)
        var lines = normalized.unicodeScalars.split(separator: "\n", omittingEmptySubsequences: false).dropFirst(0)
        var blocks: [Self] = []
        while let line = lines.popFirst() {
            let trimmed = line.drop(while: JSWhitespace.isWhitespace)
            if trimmed.isEmpty { continue }
            if trimmed.starts(with: "```".unicodeScalars) {
                var body: [String] = []
                while let raw = lines.popFirst() {
                    if raw.drop(while: JSWhitespace.isWhitespace).starts(with: "```".unicodeScalars) { break }
                    body.append(String(raw))
                }
                blocks.append(.code(body.joined(separator: "\n")))
            } else if let heading = heading(line) {
                blocks.append(heading)
            } else if isRule(trimmed) {
                blocks.append(.rule)
            } else if line.contains("|"), let next = lines.first, isTableSeparator(next) {
                _ = lines.popFirst()
                var rows: [[String]] = []
                while let row = lines.first, row.contains("|") {
                    rows.append(cells(row))
                    _ = lines.popFirst()
                }
                blocks.append(.table(head: cells(line), rows: rows))
            } else if trimmed.first == ">" {
                var body = [quoteLine(trimmed)]
                while let next = lines.first?.drop(while: JSWhitespace.isWhitespace), next.first == ">" {
                    body.append(quoteLine(next))
                    _ = lines.popFirst()
                }
                blocks.append(.quote(body.joined(separator: " ")))
            } else if let item = listItem(line) {
                var items: [String] = []
                var current = [item.text]
                while let next = lines.first {
                    if let sibling = listItem(next), (item.ordinal != nil) == (sibling.ordinal != nil) {
                        items.append(current.joined(separator: " "))
                        current = [sibling.text]
                    } else {
                        let indent = next.prefix(while: JSWhitespace.isWhitespace)
                        guard indent.prefix(2).count == 2, indent.endIndex != next.endIndex else { break }
                        current.append(JSWhitespace.trim(String(next)))
                    }
                    _ = lines.popFirst()
                }
                items.append(current.joined(separator: " "))
                // JavaScript represents list starts as numbers. Keep exact
                // integers in its safe range and leave headroom for offsets.
                let start = item.ordinal.map { digits in
                    guard let value = Int(digits), value <= 9_007_199_254_740_991 else { return 1 }
                    return value
                }
                blocks.append(.list(start: start, items: items))
            } else {
                // Rules and tables only open at a block boundary upstream.
                var body = [JSWhitespace.trim(String(trimmed))]
                while let next = lines.first {
                    let leading = next.drop(while: JSWhitespace.isWhitespace)
                    guard !leading.isEmpty, leading.first != ">",
                          !leading.starts(with: "```".unicodeScalars),
                          listItem(next) == nil, heading(next) == nil else { break }
                    body.append(JSWhitespace.trim(String(leading)))
                    _ = lines.popFirst()
                }
                blocks.append(.paragraph(body.joined(separator: " ")))
            }
        }
        return blocks
    }

    private typealias Line = String.UnicodeScalarView.SubSequence

    private static func heading(_ line: Line) -> Self? {
        let marks = line.prefix(5).prefix { $0 == "#" }
        let rest = line.dropFirst(marks.count)
        guard (1...4).contains(marks.count), let text = followingSpace(rest) else { return nil }
        return .heading(marks.count, text)
    }

    private static func listItem(_ line: Line) -> (ordinal: String?, text: String)? {
        let indent = line.prefix(4).prefix(while: JSWhitespace.isWhitespace)
        guard indent.count <= 3 else { return nil }
        let rest = line.dropFirst(indent.count)
        if let marker = rest.first, marker == "-" || marker == "*" || marker == "+" {
            return followingSpace(rest.dropFirst()).map { (nil, $0) }
        }
        let digits = rest.prefix { (48...57).contains($0.value) }
        guard !digits.isEmpty else { return nil }
        let suffix = rest[digits.endIndex...]
        guard let marker = suffix.first, marker == "." || marker == ")",
              let text = followingSpace(suffix.dropFirst()) else { return nil }
        return (String(digits), text)
    }

    private static func followingSpace(_ line: Line) -> String? {
        guard let first = line.first, JSWhitespace.isWhitespace(first) else { return nil }
        let text = line.drop(while: JSWhitespace.isWhitespace)
        // JavaScript's dot also excludes these two line separators.
        guard !text.contains(where: { $0.value == 0x2028 || $0.value == 0x2029 }) else { return nil }
        return String(text)
    }

    private static func quoteLine(_ line: Line) -> String {
        let content = line.dropFirst()
        return String(content.first.map(JSWhitespace.isWhitespace) == true ? content.dropFirst() : content)
    }

    private static func isRule(_ line: Line) -> Bool {
        guard let mark = line.first, mark == "-" || mark == "*" || mark == "_" else { return false }
        var count = 0
        for scalar in line where !JSWhitespace.isWhitespace(scalar) {
            guard scalar == mark else { return false }
            count += 1
        }
        return count >= 3
    }

    private static func isTableSeparator(_ line: Line) -> Bool {
        let columns = strippedPipes(line).split(separator: "|", omittingEmptySubsequences: false)
        return !columns.isEmpty && columns.allSatisfy { column in
            var marks = trimmed(column)
            if marks.first == ":" { marks = marks.dropFirst() }
            if marks.last == ":" { marks = marks.dropLast() }
            return !marks.isEmpty && marks.allSatisfy { $0 == "-" }
        }
    }

    private static func cells(_ line: Line) -> [String] {
        strippedPipes(line).split(separator: "|", omittingEmptySubsequences: false).map { String(trimmed($0)) }
    }

    private static func strippedPipes(_ line: Line) -> Line {
        var inner = trimmed(line)
        if inner.first == "|" { inner = inner.dropFirst() }
        if inner.last == "|" { inner = inner.dropLast() }
        return inner
    }

    private static func trimmed(_ line: Line) -> Line {
        let start = line.drop(while: JSWhitespace.isWhitespace)
        guard let end = start.lastIndex(where: { !JSWhitespace.isWhitespace($0) }) else { return start }
        return start[...end]
    }
}
