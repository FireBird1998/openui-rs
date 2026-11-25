// Observe the frames SwiftUI actually drew, through the normal library and
// LiveProgram. Chrome adds a background probe without changing child sizes.
import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import Testing

@MainActor final class LayoutPicture {
    private var frames: [String: CGRect] = [:]
    private var bitmap: NSBitmapImageRep?
    var fallback: [String] = []

    init(_ source: String, width: CGFloat = 240, height: CGFloat = 160,
         mode: IntelligentUIMode = .light, font: Font? = nil,
         component: @escaping @MainActor @Sendable (Element, String) -> AnyView? = { _, _ in nil }) async throws {
        let live = try LiveProgram(library: try await Core().intelligentUILibrary())
        try await live.show(response: source, streaming: false)
        #expect(live.errors.map(\.message) == [])
        let drawing = ElementDrawing(component: component, chrome: { view, _, path in
            AnyView(view.background {
                GeometryReader { bounds in
                    let _ = self.frames[path] = bounds.frame(in: .named("picture"))
                    Color.clear
                }
            })
        }, fallback: { element, _ in
            self.fallback.append(element.typeName)
            return AnyView(EmptyView())
        })
        let view = ElementView(live: live).environment(\.elementDrawing, drawing)
            .font(font)
            .frame(width: width, height: height, alignment: .topLeading)
            .coordinateSpace(name: "picture").intelligentUIMode(mode)
            // ImageRenderer has no window to supply CSS viewport dimensions.
            .intelligentUIViewport(CGSize(width: width, height: height))
        let image = try #require(ImageRenderer(content: view).cgImage)
        bitmap = NSBitmapImageRep(cgImage: image)
    }

    func rect(_ path: String) throws -> CGRect { try #require(frames[path]) }
}

extension LayoutPicture {
    // Compare paints in the same output colour space/profile ImageRenderer
    // chose on this Mac. Geometry is compared in points separately.
    static func swatch(_ color: Color, over background: Color = .clear) throws -> NSColor {
        let reference = color.frame(width: 4, height: 4).background(background)
        let image = try #require(ImageRenderer(content: reference).cgImage)
        return try #require(NSBitmapImageRep(cgImage: image).colorAt(x: 2, y: 2)?.usingColorSpace(.sRGB))
    }

    func pixel(_ x: Int, _ y: Int) throws -> NSColor {
        let size = "\(bitmap?.pixelsWide ?? 0)x\(bitmap?.pixelsHigh ?? 0) px"
        let raw = try #require(bitmap?.colorAt(x: x, y: y), "No pixel at (\(x), \(y)) in a \(size) picture")
        return try #require(raw.usingColorSpace(.sRGB), "Pixel (\(x), \(y)) = \(raw) has no sRGB form")
    }
}

extension LayoutPicture {
    func renderedPixels() throws -> Data {
        let image = try #require(bitmap)
        let bytes = try #require(image.bitmapData)
        return Data(bytes: bytes, count: image.bytesPerRow * image.pixelsHigh)
    }
}
