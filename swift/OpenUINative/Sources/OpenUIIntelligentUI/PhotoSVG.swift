import AppKit

/// AppKit rasterizes a new SVG built from static drawing elements. The
/// original XML, CSS, linking attributes and resource URLs never reach it.
final class PhotoSVG: NSObject, XMLParserDelegate {
    private static let svgNamespace = "http://www.w3.org/2000/svg"
    private static let tags: Set<String> = [
        "g", "defs", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon",
        "linearGradient", "radialGradient", "stop", "clipPath", "text", "tspan",
    ]
    private static let attributes: Set<String> = [
        "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "fx", "fy", "fr", "rx", "ry", "r",
        "dx", "dy", "width", "height", "d", "points", "viewBox", "preserveAspectRatio", "transform",
        "fill", "fill-opacity", "fill-rule", "stroke", "stroke-width", "stroke-opacity", "stroke-dasharray",
        "stroke-dashoffset", "stroke-linecap", "stroke-linejoin", "stroke-miterlimit",
        "color", "opacity", "display", "visibility", "vector-effect", "gradientUnits", "gradientTransform",
        "spreadMethod", "offset", "stop-color", "stop-opacity", "clip-path", "clip-rule", "clipPathUnits",
        "font-size", "font-weight", "font-style", "text-anchor", "dominant-baseline",
    ]
    private static let metadataTags: Set<String> = ["title", "desc", "metadata"]
    private static let editorAttributes: Set<String> = [
        "inkscape:version", "inkscape:label", "inkscape:groupmode", "sodipodi:docname",
    ]
    private static let rootGeometry: Set<String> = ["width", "height", "viewBox"]
    private static let definitionTags: Set<String> = ["linearGradient", "radialGradient", "clipPath"]
    private static let paintLinks: Set<String> = ["fill", "stroke", "clip-path"]
    private static let valueCharacters = CharacterSet(charactersIn:
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 #.,+-()%\t\r\n")
    private static let idCharacters = CharacterSet(charactersIn:
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-.")
    private let size: PhotoSize
    private var stack: [String] = []
    private var documentNamespace = ""
    private var metadataDepth = 0
    private var ids: [String: String] = [:]
    private var paintReferences: [(attribute: String, id: String)] = []
    private var definitionDepth = 0
    private var nodes = 0
    private var output = ""
    private var outputSize = CGSize.zero
    private var refused = false

    private init(size: PhotoSize) { self.size = size }

    static func decode(_ data: Data, size: PhotoSize) -> CGImage? {
        guard data.count <= PhotoLimits.svgInputBytes, let xml = String(data: data, encoding: .utf8),
              !xml.contains("\0"), let headerChecked = strippingBareDoctype(from: xml) else { return nil }
        let sanitized = PhotoSVG(size: size)
        let parser = XMLParser(data: Data(headerChecked.utf8))
        parser.shouldProcessNamespaces = true
        parser.shouldResolveExternalEntities = false
        parser.externalEntityResolvingPolicy = .never
        parser.delegate = sanitized
        guard parser.parse(), !sanitized.refused, sanitized.outputSize != .zero,
              sanitized.paintReferences.allSatisfy({ reference in
                  guard let target = sanitized.ids[reference.id] else { return false }
                  return reference.attribute == "clip-path" ? target == "clipPath"
                      : target == "linearGradient" || target == "radialGradient"
              }),
              let image = NSImage(data: Data(sanitized.output.utf8)),
              let context = PhotoRaster.bitmap(width: Int(sanitized.outputSize.width),
                                               height: Int(sanitized.outputSize.height))
        else { return nil }
        NSGraphicsContext.saveGraphicsState()
        defer { NSGraphicsContext.restoreGraphicsState() }
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
        image.draw(in: CGRect(origin: .zero, size: sanitized.outputSize))
        return context.makeImage()
    }

    // The only removable declaration has no external ID or internal subset.
    // Require it in the prolog and reject every further DOCTYPE before XMLParser.
    private static func strippingBareDoctype(from xml: String) -> String? {
        guard let declaration = xml.range(of: "<!DOCTYPE") else { return xml }
        var prolog = xml[..<declaration.lowerBound]
        if prolog.first == "\u{FEFF}" { prolog = prolog.dropFirst() }
        for _ in prolog.utf8 {
            prolog = prolog.drop(while: { character in
                character.unicodeScalars.allSatisfy { " \t\r\n".unicodeScalars.contains($0) }
            })
            if prolog.isEmpty { break }
            let terminator: String
            if prolog.hasPrefix("<!--") { terminator = "-->" }
            else if prolog.hasPrefix("<?xml") { terminator = "?>" }
            else { return nil }
            guard let end = prolog.range(of: terminator) else { return nil }
            prolog = prolog[end.upperBound...]
        }
        guard prolog.isEmpty,
              let bare = xml[declaration.lowerBound...].range(
                  of: #"<!DOCTYPE[ \t\r\n]+svg[ \t\r\n]*>"#, options: [.anchored, .regularExpression]),
              !xml[bare.upperBound...].contains("<!DOCTYPE") else { return nil }
        return String(xml[..<bare.lowerBound]) + xml[bare.upperBound...]
    }

    func parser(_ parser: XMLParser, didStartElement name: String, namespaceURI: String?,
                qualifiedName: String?, attributes: [String: String]) {
        nodes += 1
        guard nodes <= PhotoLimits.svgNodes, stack.count + metadataDepth < PhotoLimits.svgDepth else {
            refuse(parser)
            return
        }
        let namespace = namespaceURI ?? ""
        if stack.isEmpty {
            guard name == "svg", namespace == Self.svgNamespace || namespace.isEmpty,
                  let dimensions = intrinsicSize(attributes) else {
                refuse(parser)
                return
            }
            documentNamespace = namespace
            let edge = size.longestEdge(width: Int(ceil(dimensions.width)), height: Int(ceil(dimensions.height)),
                                        upscale: true)
            let longestSide = max(dimensions.width, dimensions.height)
            outputSize = CGSize(width: max(1, (dimensions.width / longestSide * CGFloat(edge)).rounded()),
                                height: max(1, (dimensions.height / longestSide * CGFloat(edge)).rounded()))
            let viewBox = attributes["viewBox"] ?? "0 0 \(dimensions.width) \(dimensions.height)"
            output += "<svg xmlns=\"\(Self.svgNamespace)\" width=\"\(outputSize.width)\""
            output += " height=\"\(outputSize.height)\" viewBox=\"\(viewBox)\""
        } else if metadataDepth > 0 || (namespace == documentNamespace && Self.metadataTags.contains(name))
                    || (namespace == "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" && name == "namedview") {
            metadataDepth += 1
            return
        } else {
            guard namespace == documentNamespace, Self.tags.contains(name) else {
                refuse(parser)
                return
            }
            output += "<\(name)"
        }

        if Self.definitionTags.contains(name) { definitionDepth += 1 }
        guard let values = Self.paintAttributes(attributes, root: name == "svg") else {
            refuse(parser)
            return
        }
        for (key, raw) in values {
            let value = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            if key == "id" {
                guard Self.isID(raw), ids.updateValue(name, forKey: raw) == nil else {
                    refuse(parser)
                    return
                }
            } else if value.lowercased().contains("url") {
                let id = String(value.dropFirst(5).dropLast())
                guard definitionDepth == 0, Self.paintLinks.contains(key),
                      value.hasPrefix("url(#"), value.hasSuffix(")"), Self.isID(id) else {
                    refuse(parser)
                    return
                }
                paintReferences.append((key, id))
            } else if !Self.isPlainValue(value) {
                refuse(parser)
                return
            }
            output += " \(key)=\"\(value)\""
        }
        output += ">"
        stack.append(name)
    }

    private static func paintAttributes(_ attributes: [String: String], root: Bool) -> [String: String]? {
        var values: [String: String] = [:]
        for (key, value) in attributes {
            if key == "style" || editorAttributes.contains(key)
                || (root && (rootGeometry.contains(key) || key == "version" || key == "baseProfile")) { continue }
            guard Self.attributes.contains(key) || key == "id" else { return nil }
            values[key] = value
        }
        if let style = attributes["style"] {
            let declarations = style.split(separator: ";")
            guard declarations.count <= PhotoLimits.svgStyleDeclarations else { return nil }
            for declaration in declarations {
                let trimmed = declaration.trimmingCharacters(in: .whitespacesAndNewlines)
                if trimmed.isEmpty { continue }
                let pair = trimmed.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
                guard pair.count == 2, let property = pair.first, let raw = pair.last else { return nil }
                let key = property.trimmingCharacters(in: .whitespacesAndNewlines)
                let value = raw.trimmingCharacters(in: .whitespacesAndNewlines)
                // Validate every declaration before replacing earlier values.
                // In particular, priority markers would change which value wins.
                guard Self.attributes.contains(key), !(root && rootGeometry.contains(key)),
                      isPlainValue(value) else { return nil }
                values[key] = value
            }
        }
        return values
    }

    func parser(_ parser: XMLParser, didEndElement name: String, namespaceURI: String?, qualifiedName: String?) {
        if metadataDepth > 0 {
            metadataDepth -= 1
        } else if let tag = stack.popLast() {
            if Self.definitionTags.contains(tag) { definitionDepth -= 1 }
            output += "</\(tag)>"
        }
    }

    func parser(_ parser: XMLParser, foundProcessingInstructionWithTarget target: String, data: String?) {
        refuse(parser)
    }

    func parser(_ parser: XMLParser, foundCharacters text: String) {
        appendText(text)
    }

    func parser(_ parser: XMLParser, foundCDATA data: Data) {
        guard let text = String(data: data, encoding: .utf8) else {
            refuse(parser)
            return
        }
        appendText(text)
    }

    private func appendText(_ text: String) {
        guard metadataDepth == 0, let tag = stack.last, tag == "text" || tag == "tspan" else { return }
        output += text.replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: ">", with: "&gt;")
    }

    private func refuse(_ parser: XMLParser) {
        refused = true
        parser.abortParsing()
    }

    private static func isID(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= PhotoLimits.svgIDBytes
            && value.unicodeScalars.allSatisfy(idCharacters.contains)
    }

    private static func isPlainValue(_ value: String) -> Bool {
        !value.isEmpty && value.unicodeScalars.allSatisfy(valueCharacters.contains)
    }

    private static func viewBox(_ value: String) -> CGSize? {
        let parts = value.split { $0.isWhitespace || $0 == "," }
        guard parts.count == 4 else { return nil }
        let numbers = parts.compactMap { Double($0) }
        guard value.unicodeScalars.allSatisfy(valueCharacters.contains), numbers.count == parts.count,
              numbers.allSatisfy({ $0.isFinite && abs($0) <= PhotoLimits.svgViewBoxMagnitude }),
              let width = numbers.dropFirst(2).first, let height = numbers.last, width > 0, height > 0
        else { return nil }
        return CGSize(width: width, height: height)
    }

    private func intrinsicSize(_ attributes: [String: String]) -> CGSize? {
        let viewBox = attributes["viewBox"].flatMap(Self.viewBox)
        guard attributes["viewBox"] == nil || viewBox != nil else { return nil }
        let ratio = viewBox.map { $0.width / $0.height } ?? 2
        func length(_ value: String?) -> CGFloat? {
            guard let value else { return nil }
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            let scalar = trimmed.hasSuffix("px") ? String(trimmed.dropLast(2)) : trimmed
            return Double(scalar).map { CGFloat($0) }
        }
        let width = length(attributes["width"])
        let height = length(attributes["height"])
        guard attributes["width"] == nil || width != nil, attributes["height"] == nil || height != nil
        else { return nil }
        let w = width ?? height.map { $0 * ratio } ?? min(300, 150 * ratio)
        let h = height ?? w / ratio
        guard w.isFinite, h.isFinite, w > 0, h > 0, w <= CGFloat(PhotoLimits.sourceAxis),
              h <= CGFloat(PhotoLimits.sourceAxis), w * h <= CGFloat(PhotoLimits.sourcePixels)
        else { return nil }
        return CGSize(width: w, height: h)
    }
}
