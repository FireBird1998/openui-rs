// The vector paths and wire names come from lucide-react 0.562.0. See
// Lucide/LICENSE.txt and scripts/lucide-artwork.mjs.
import SwiftUI

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
            let trimmed = JSWhitespace.trim(name)
            guard !trimmed.isEmpty else { return nil }
            func replace(_ text: String, _ regex: String, _ replacement: String) -> String {
                text.replacingOccurrences(of: regex, with: replacement, options: .regularExpression)
            }
            var base = ""
            var separator = false
            for scalar in trimmed.unicodeScalars {
                if scalar == "_" || JSWhitespace.isWhitespace(scalar) {
                    if !separator { base.append("-") }
                    separator = true
                } else {
                    base.unicodeScalars.append(scalar)
                    separator = false
                }
            }
            // A match spans the whole uppercase run. Skip interior starts
            // so a long run without a lowercase tail takes one scan.
            base = replace(base, "(?<![A-Z])([A-Z]+)([A-Z][a-z])", "$1-$2")
            base = replace(base, "([a-z0-9])([A-Z])", "$1-$2")
            base = replace(base, "([A-Za-z])([0-9])", "$1-$2").lowercased()
            for candidate in [base, replace(base, "([0-9])(?=[0-9])", "$1-"),
                              replace(base, "([0-9])x-([0-9])", "$1x$2"),
                              replace(base, "([0-9])x-([0-9])", "$1-x-$2")] {
                if let icon = match(candidate) { return icon }
            }
            // Only prefixes as long as a catalog key can match. Keep the
            // following separator so the longest possible prefix is tried.
            // JavaScript splits on the hyphen even next to a combining mark.
            var prefix = base.unicodeScalars.prefix(longestName + 1)
            while let separator = prefix.lastIndex(of: "-") {
                prefix = prefix.prefix(upTo: separator)
                if let icon = match(String(prefix)) { return icon }
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
    private final class Artwork {
        let shapes: [LucideShape]

        init(_ shapes: [LucideShape]) { self.shapes = shapes }
    }

    @MainActor private static let artworks = NSCache<NSString, Artwork>()

    @MainActor static func shapes(named name: String) -> [LucideShape]? {
        guard let catalog, let key = catalog.resolve(name) else { return nil }
        if let cached = artworks.object(forKey: key as NSString) { return cached.shapes }
        guard let markup = catalog.artwork[key], let shapes = LucideShape.read(markup) else { return nil }
        artworks.setObject(Artwork(shapes), forKey: key as NSString)
        return shapes
    }
}

struct LucideDrawing: View {
    private let shapes: [LucideShape]
    private let tint: Color

    init?(name: String?, tint: Color) {
        guard let name, let shapes = LucideGlyph.shapes(named: name) else { return nil }
        self.shapes = shapes
        self.tint = tint
    }

    var body: some View {
        Canvas { context, size in
            let scale = min(size.width, size.height) / 24
            context.translateBy(x: (size.width - 24 * scale) / 2, y: (size.height - 24 * scale) / 2)
            context.scaleBy(x: scale, y: scale)
            let stroke = StrokeStyle(lineWidth: 1.8, lineCap: .round, lineJoin: .round)
            for shape in shapes {
                if shape.filled { context.fill(shape.path, with: .color(tint)) }
                context.stroke(shape.path, with: .color(tint), style: stroke)
            }
        }
        .accessibilityHidden(true)
        .allowsHitTesting(false)
    }
}
