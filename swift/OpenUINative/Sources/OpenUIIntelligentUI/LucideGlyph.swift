// The vector paths and wire names come from lucide-react 0.562.0. See
// Lucide/LICENSE.txt and scripts/lucide-artwork.mjs.
import AppKit

enum LucideGlyph {
    private struct Catalog: Decodable {
        let artwork: [String: String]
        let names: [String: String]
        let permutations: [String: String]

        func match(_ name: String) -> String? {
            guard name.utf8.prefix(longestName + 1).count <= longestName else { return nil }
            return names[name] ?? permutations[name.components(separatedBy: "-").sorted().joined(separator: "-")]
        }

        func resolve(_ name: String) -> String? {
            if let exact = match(name) { return exact }
            func replace(_ text: String, _ regex: String, _ replacement: String) -> String {
                text.replacingOccurrences(of: regex, with: replacement, options: .regularExpression)
            }
            var base = ""
            var separator = false
            for scalar in JSWhitespace.trim(name).unicodeScalars {
                if scalar == "_" || JSWhitespace.isWhitespace(scalar) {
                    if !separator { base.append("-") }
                    separator = true
                } else {
                    base.unicodeScalars.append(scalar)
                    separator = false
                }
            }
            base = replace(base, "([A-Z]+)([A-Z][a-z])", "$1-$2")
            base = replace(base, "([a-z0-9])([A-Z])", "$1-$2")
            base = replace(base, "([A-Za-z])([0-9])", "$1-$2").lowercased()
            for candidate in [base, replace(base, "([0-9])(?=[0-9])", "$1-"),
                              replace(base, "([0-9])x-([0-9])", "$1x$2"),
                              replace(base, "([0-9])x-([0-9])", "$1-x-$2")] {
                if let icon = match(candidate) { return icon }
            }
            // Only prefixes as long as a catalog key can match. Keep the
            // following separator so the longest possible prefix is tried.
            base = String(base.prefix(longestName + 1))
            while let separator = base.lastIndex(of: "-") {
                base.removeSubrange(separator...)
                if let icon = match(base) { return icon }
            }
            return names["circle-dot"]
        }
    }

    private static let catalog: Catalog? = {
        guard let url = Bundle.module.url(forResource: "catalog", withExtension: "json", subdirectory: "Lucide"),
              let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(Catalog.self, from: data)
    }()
    private static let longestName = catalog?.names.keys.map { $0.utf8.count }.max() ?? 0
    @MainActor private static let images = NSCache<NSString, NSImage>()

    @MainActor static func image(named name: String) -> NSImage? {
        guard let catalog, let key = catalog.resolve(name) else { return nil }
        if let cached = images.object(forKey: key as NSString) { return cached }
        guard let paths = catalog.artwork[key] else { return nil }
        let svg = """
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">\(paths)</svg>
        """
        guard let image = NSImage(data: Data(svg.utf8)) else { return nil }
        image.isTemplate = true
        images.setObject(image, forKey: key as NSString)
        return image
    }
}
