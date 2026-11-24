// The inline language in components/Markdown/markdownBlocks.tsx. A token's
// first matching delimiter wins, and code is literal. HTML is ordinary text.
import Foundation

enum InlineText {
    private struct Token {
        let body: Range<String.Index>
        let end: String.Index
        let intent: InlinePresentationIntent
        var destination: Range<String.Index>?
    }

    // Look ahead once per delimiter, even for a long line of unmatched "[".
    // Indices come from the same scalar view, so a delimiter can precede a
    // combining mark without losing either scalar.
    private static func tokens(_ text: String) -> [String.Index: Token] {
        let s = text.unicodeScalars
        var result: [String.Index: Token] = [:]
        var next: [Unicode.Scalar: String.Index] = [:]
        var nextURLStop = s.endIndex
        var linkSuffix: Token?
        for i in s.indices.reversed() {
            let c = s[i], after = s.index(after: i)
            if c == "\n" { next.removeAll(keepingCapacity: true); linkSuffix = nil }
            if c == ")" || JSWhitespace.isWhitespace(c) { nextURLStop = i }
            if c == "]" {
                linkSuffix = nil
                if after < s.endIndex, s[after] == "(" {
                    let start = s.index(after: after)
                    if start < nextURLStop, nextURLStop < s.endIndex, s[nextURLStop] == ")" {
                        linkSuffix = Token(body: i..<i, end: s.index(after: nextURLStop), intent: [],
                                          destination: start..<nextURLStop)
                    }
                }
            } else if c == "[", let link = linkSuffix, after < link.body.lowerBound {
                result[i] = Token(body: after..<link.body.lowerBound, end: link.end, intent: [], destination: link.destination)
            } else if c == "*" || c == "_" || c == "~" || c == "`" {
                if c != "`", after < s.endIndex, s[after] == c,
                   let close = nextDoubleBodyEnd(s, after: after, delimiter: c) {
                    result[i] = Token(body: s.index(after: after)..<close, end: s.index(close, offsetBy: 2),
                                      intent: c == "~" ? .strikethrough : .stronglyEmphasized)
                } else if let end = next[c], after < end,
                          c == "`" || (c != "~" && !JSWhitespace.isWhitespace(s[after])) {
                    result[i] = Token(body: after..<end, end: s.index(after: end), intent: c == "`" ? .code : .emphasized)
                }
                next[c] = i
            }
        }
        return result
    }

    private static func nextDoubleBodyEnd(_ s: String.UnicodeScalarView, after: String.Index,
                                          delimiter: Unicode.Scalar) -> String.Index? {
        let start = s.index(after: after)
        guard start < s.endIndex, s[start] != delimiter,
              let close = s[start...].firstIndex(where: { $0 == delimiter || $0 == "\n" }),
              s[close] == delimiter else { return nil }
        let second = s.index(after: close)
        return second < s.endIndex && s[second] == delimiter ? close : nil
    }

    // CSS collapses whitespace after markup has been parsed. Preserve the
    // attributes of the first space and trim spaces at either end of a line.
    static func whitespace(_ value: AttributedString, preserveLines: Bool) -> AttributedString {
        var result = AttributedString()
        var space: AttributeContainer?
        var atStart = true
        for run in value.runs {
            let chars = value[run.range].characters
            var start = chars.startIndex
            func appendWord(through end: AttributedString.Index) {
                guard start < end else { return }
                if let space { result.append(AttributedString(" ", attributes: space)) }
                space = nil
                result.append(AttributedString(value[start..<end]))
                atStart = false
            }
            for i in chars.indices {
                let c = chars[i]
                let newline = c == "\r" || c == "\n" || c == "\r\n"
                guard newline || c == " " || c == "\t" || c == "\u{c}" else { continue }
                appendWord(through: i)
                if preserveLines && newline {
                    result.append(AttributedString("\n", attributes: run.attributes))
                    atStart = true
                    space = nil
                } else if !atStart && space == nil {
                    space = run.attributes
                }
                start = chars.index(after: i)
            }
            appendWord(through: chars.endIndex)
        }
        return result
    }

    static func parse(_ text: String) -> AttributedString {
        let matches = tokens(text)
        let scalars = text.unicodeScalars
        struct Span {
            var range: Range<String.Index>
            var intent: InlinePresentationIntent
            var link: URL?
        }
        var pending = [Span(range: scalars.startIndex..<scalars.endIndex, intent: [], link: nil)]
        var out = AttributedString()
        func append(_ range: Range<String.Index>, _ span: Span) {
            guard !range.isEmpty else { return }
            var part = AttributedString(String(scalars[range]))
            if !span.intent.isEmpty { part.inlinePresentationIntent = span.intent }
            part.link = span.link
            out.append(part)
        }
        while let span = pending.popLast() {
            var i = span.range.lowerBound
            while i < span.range.upperBound {
                if let token = matches[i], token.end <= span.range.upperBound {
                    append(span.range.lowerBound..<i, span)
                    if token.end < span.range.upperBound {
                        pending.append(Span(range: token.end..<span.range.upperBound, intent: span.intent, link: span.link))
                    }
                    let destination = token.destination.flatMap { webURL(String(scalars[$0])) } ?? span.link
                    let inner = Span(range: token.body, intent: span.intent.union(token.intent), link: destination)
                    if token.intent == .code { append(inner.range, inner) } else { pending.append(inner) }
                    break
                }
                i = scalars.index(after: i)
            }
            if i == span.range.upperBound { append(span.range, span) }
        }
        return out
    }
}
