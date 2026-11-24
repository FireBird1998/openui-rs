// shared/url.ts uses new URL without a base, accepting only http and https.
// Normalize their special-scheme authority before handing the URL to macOS.
import Darwin
import Foundation

func webURL(_ source: String) -> URL? {
    let trimmed = JSWhitespace.trim(source).trimmingCharacters(in: CharacterSet(charactersIn: "\u{0}"..." "))
    let text = trimmed.filter { $0 != "\t" && $0 != "\n" && $0 != "\r" && $0 != "\r\n" }
    guard let colon = text.firstIndex(of: ":") else { return nil }
    let scheme = text[..<colon].lowercased()
    guard scheme == "http" || scheme == "https" else { return nil }
    let rest = text[text.index(after: colon)...].drop { $0 == "/" || $0 == "\\" }
    let authority = rest.prefix { !"/\\?#".contains($0) }
    let hostPort = authority.split(separator: "@", omittingEmptySubsequences: false).last ?? ""
    guard !hostPort.isEmpty else { return nil }
    // Foundation returns a nil port for an integer too large to represent.
    if let colon = hostPort.lastIndex(of: ":"), !hostPort[colon...].contains("]") {
        let port = hostPort[hostPort.index(after: colon)...]
        guard port.isEmpty || port.utf8.allSatisfy({ (48...57).contains($0) }) && UInt16(port) != nil else { return nil }
    }
    let path = rest[authority.endIndex...].prefix { $0 != "?" && $0 != "#" }
    let suffix = rest[path.endIndex...]
    let address = scheme + "://" + authority + path.replacingOccurrences(of: "\\", with: "/") + suffix
    // The byte-based initializer encodes Unicode without re-escaping an
    // existing percent triplet in the same component.
    guard let url = URL(dataRepresentation: Data(address.utf8), relativeTo: nil),
          var parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
          let host = parts.host, !host.isEmpty else { return nil }
    if host.hasPrefix("["), host.hasSuffix("]") {
        var ip = in6_addr()
        // Brackets and IPv6 syntax must be literal in the authority. A
        // percent-decoded domain cannot turn into a bracketed address.
        guard hostPort.hasPrefix("["), !hostPort.contains("%"),
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
            guard inet_aton(String(domain), &ip) == 1 else { return nil }
            let n = UInt32(bigEndian: ip.s_addr)
            parts.host = [24, 16, 8, 0].map { String((n >> $0) & 255) }.joined(separator: ".")
        } else {
            parts.host = host.lowercased()
        }
    }
    return parts.url
}
