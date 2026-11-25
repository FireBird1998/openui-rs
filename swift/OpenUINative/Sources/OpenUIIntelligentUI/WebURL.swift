// shared/url.ts uses new URL without a base, accepting only http and https.
// Normalize their special-scheme authority before handing the URL to macOS.
import Darwin
import Foundation

func webURL(_ source: String) -> URL? {
    let trimmed = JSWhitespace.trim(source).trimmingCharacters(in: CharacterSet(charactersIn: "\u{0}"..." "))
    // URL punctuation separates fields even when a combining mark follows it.
    let text = String(trimmed.unicodeScalars.filter { $0 != "\t" && $0 != "\n" && $0 != "\r" }).unicodeScalars
    guard let colon = text.firstIndex(of: ":") else { return nil }
    let scheme = String(text[..<colon]).lowercased()
    guard scheme == "http" || scheme == "https" else { return nil }
    let rest = text[text.index(after: colon)...].drop { $0 == "/" || $0 == "\\" }
    let authority = rest.prefix { !"/\\?#".unicodeScalars.contains($0) }
    guard let hostPort = authority.split(separator: "@", omittingEmptySubsequences: false).last,
          !hostPort.isEmpty else { return nil }
    // Foundation returns a nil port for an integer too large to represent.
    if let colon = hostPort.lastIndex(of: ":"), !hostPort[colon...].contains("]") {
        let port = hostPort[hostPort.index(after: colon)...]
        guard port.isEmpty || port.allSatisfy({ (48...57).contains($0.value) }) && UInt16(String(port)) != nil else { return nil }
    }
    let path = rest[authority.endIndex...].prefix { $0 != "?" && $0 != "#" }
    let suffix = rest[path.endIndex...]
    let address = scheme + "://" + String(authority)
        + String(path).replacingOccurrences(of: "\\", with: "/", options: .literal) + String(suffix)
    // The byte-based initializer encodes Unicode without re-escaping an
    // existing percent triplet in the same component.
    guard let url = URL(dataRepresentation: Data(address.utf8), relativeTo: nil),
          var parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
          let host = parts.host, !host.isEmpty else { return nil }
    if host.hasPrefix("["), host.hasSuffix("]") {
        var ip = in6_addr()
        // Brackets and IPv6 syntax must be literal in the authority. A
        // percent-decoded domain cannot turn into a bracketed address.
        guard hostPort.first == "[", !hostPort.contains("%"),
              inet_pton(AF_INET6, String(host.dropFirst().dropLast()), &ip) == 1 else { return nil }
    } else {
        guard !host.unicodeScalars.contains(where: {
            $0.value <= 32 || $0.value == 127 || "#/%:?@[\\]<>^|".unicodeScalars.contains($0)
        }) else { return nil }
        let domain = host.hasSuffix(".") ? host.dropLast() : host[...]
        let last = domain.split(separator: ".", omittingEmptySubsequences: false).last ?? ""
        let hex = last.lowercased().hasPrefix("0x") && last.dropFirst(2).allSatisfy(\.isHexDigit)
        if !last.isEmpty && (hex || last.utf8.allSatisfy({ (48...57).contains($0) })) {
            var ip = in_addr()
            // A hexadecimal IPv4 component with no digits is zero in the
            // browser URL parser. inet_aton requires at least one digit.
            let address = domain.split(separator: ".", omittingEmptySubsequences: false)
                .map { $0.lowercased() == "0x" ? "0" : String($0) }.joined(separator: ".")
            guard inet_aton(address, &ip) == 1 else { return nil }
            let n = UInt32(bigEndian: ip.s_addr)
            parts.host = [24, 16, 8, 0].map { String((n >> $0) & 255) }.joined(separator: ".")
        } else {
            parts.host = host.lowercased()
        }
    }
    return parts.url
}
