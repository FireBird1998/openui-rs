// Observe the frames SwiftUI actually drew, through the normal library and
// LiveProgram. Chrome adds a background probe without changing child sizes.
import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

@MainActor final class LayoutPicture {
    private var frames: [String: CGRect] = [:]
    private var bitmap: NSBitmapImageRep?
    var fallback: [String] = []

    init(_ source: String, width: CGFloat = 240, height: CGFloat = 160,
         mode: IntelligentUIMode = .light, font: Font? = nil,
         component: @escaping @MainActor @Sendable (Element, String) -> AnyView? = { _, _ in nil }) async throws {
        NativeTestWatchdog.beginWaiting("Core to load the Intelligent UI library for LayoutPicture")
        defer { NativeTestWatchdog.finishedWaiting() }
        let live = try LiveProgram(library: try await Core().intelligentUILibrary())
        NativeTestWatchdog.waitingFor("Core to show the LayoutPicture program")
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
        NativeTestWatchdog.waitingFor("SwiftUI to lay out and render LayoutPicture")
        let image = try #require(ImageRenderer(content: view).cgImage)
        bitmap = NSBitmapImageRep(cgImage: image)
    }

    func rect(_ path: String) throws -> CGRect { try #require(frames[path], "Missing frame at \(path)") }
}

extension LayoutPicture {
    // The 8-bit paint ImageRenderer gives a colour that is not a theme token
    // (a system colour, or one composited over a background), or a token an
    // assertion needs bit-exactly. Compare tokens with tolerance against
    // `ThemeColor.sRGB` instead. Geometry is compared in points separately.
    static func swatch(_ color: Color, over background: Color = .clear) throws -> NSColor {
        NativeTestWatchdog.beginWaiting("SwiftUI to render the reference swatch")
        defer { NativeTestWatchdog.finishedWaiting() }
        let reference = color.frame(width: 4, height: 4).background(background)
        let image = try #require(ImageRenderer(content: reference).cgImage)
        return try #require(NSBitmapImageRep(cgImage: image).sRGBColor(x: 2, y: 2))
    }

    func pixel(_ x: Int, _ y: Int) throws -> NSColor {
        let size = "\(bitmap?.pixelsWide ?? 0)x\(bitmap?.pixelsHigh ?? 0) px"
        let image = try #require(bitmap, "No picture")
        try #require(image.colorAt(x: x, y: y) != nil, "No pixel at (\(x), \(y)) in a \(size) picture")
        return try #require(image.sRGBColor(x: x, y: y), "Pixel (\(x), \(y)) in \(image.colorSpace) has no sRGB form")
    }

    func alpha(in frame: CGRect) throws -> [UInt8] {
        let bitmap = try #require(bitmap)
        let rect = frame.integral
        try #require(CGRect(x: 0, y: 0, width: bitmap.pixelsWide, height: bitmap.pixelsHigh).contains(rect))
        var result: [UInt8] = []
        for y in Int(rect.minY)..<Int(rect.maxY) {
            for x in Int(rect.minX)..<Int(rect.maxX) {
                let alpha = bitmap.colorAt(x: x, y: y)?.alphaComponent ?? 0
                result.append(UInt8(clamping: Int((alpha * 255).rounded())))
            }
        }
        return result
    }
}

extension LayoutPicture {
    func writePNG(to url: URL) throws {
        guard let png = bitmap?.representation(using: .png, properties: [:]) else {
            throw CocoaError(.fileWriteUnknown)
        }
        try png.write(to: url, options: .atomic)
    }

    func renderedPixels() throws -> Data {
        let image = try #require(bitmap)
        let bytes = try #require(image.bitmapData)
        return Data(bytes: bytes, count: image.bytesPerRow * image.pixelsHigh)
    }
}
