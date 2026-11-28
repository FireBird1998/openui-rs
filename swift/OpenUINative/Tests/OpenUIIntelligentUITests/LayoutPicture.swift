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
         mode: IntelligentUIMode = .light, font: Font? = nil, formContext: FormContext? = nil,
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
            .environment(\.formContext, formContext)
            .font(font)
            .frame(width: width, height: height, alignment: .topLeading)
            .coordinateSpace(name: "picture").intelligentUIMode(mode)
            // ImageRenderer has no window to supply CSS viewport dimensions.
            .intelligentUIViewport(CGSize(width: width, height: height))
        NativeTestWatchdog.waitingFor("SwiftUI to lay out and render LayoutPicture")
        bitmap = try Self.capture(view)
    }

    // Every picture, and every swatch, is captured by replaying ImageRenderer's
    // drawing through CoreGraphics into an explicit 8-bit sRGB bitmap
    // (premultiplied RGBA), not read from `ImageRenderer.cgImage`.
    //
    // Why: `cgImage` is not deterministic within a process. On the main Mac
    // (M3 Pro, display awake) the first two Swift Charts renders of a fresh
    // process, and some later renders, draw anti-aliased edges 1/255 apart
    // from every other render of the same view, in G, B and alpha, never in a
    // solid fill. Its output is already 8-bit sRGB, so this is not colour
    // matching. Swift Testing interleaves the main-actor tests, so which test
    // gets those "cold" renders changes from run to run, and an exact
    // comparison of two pictures fails when a cold and a warm render land in
    // one pair (PolarChartDrawingTests' pie and donut, ChartDrawingTests' bar
    // percent). Measured: the two chart suites alone in fresh processes failed
    // 8 of 18 times through `cgImage` (3 of 10 with the display asleep) and
    // passed 10 of 10 through this replay, which was bit-identical from the
    // first render of a process on.
    //
    // Why the worker and GitHub's runner never failed: the worker does fail,
    // since 556c349 made the target serial. Probed on the worker (2026-11-28,
    // over SSH, no screen): pie and donut have no cold variant there (5 of 5
    // fresh processes identical from the first render), but bar's first two
    // renders differ from later ones by 1/255 in alpha at one edge pixel, as
    // here. The parallel order hid it (1d57069 passed on the worker); the
    // serial order is fixed and puts that boundary inside the bar percent
    // pair every time (b3ac145 and 6d68706: 5 of 5 full serial runs failed at
    // ChartDrawingTests.swift:59, the two chart suites alone 3 of 3).
    // GitHub's runner passes in the same serial order (CI on 6d68706), so it
    // shows no cold variant there; it was not probed directly. Through this
    // replay the worker passed the two suites 3 of 3 and the full target with
    // no pixel failure.
    //
    // The replay rasterises edges with CoreGraphics' own coverage: on the
    // pixels this target samples, up to 64/255 away from `cgImage`'s on chart
    // edges and 103/255 on thin icon strokes (solid fills are the same). An
    // assertion that reads an edge pixel must derive its expectation from the
    // geometry, never from a `cgImage` capture (TESTING.md, "Pixel capture").
    static func capture(_ view: some View) throws -> NSBitmapImageRep {
        let renderer = ImageRenderer(content: view)
        var image: CGImage?
        renderer.render { size, draw in
            guard let space = CGColorSpace(name: CGColorSpace.sRGB),
                  let context = CGContext(data: nil, width: Int(size.width.rounded(.up)),
                                          height: Int(size.height.rounded(.up)), bitsPerComponent: 8,
                                          bytesPerRow: 0, space: space,
                                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
            draw(context)
            image = context.makeImage()
        }
        return NSBitmapImageRep(cgImage: try #require(image, "ImageRenderer gave nothing to draw"))
    }

    func rect(_ path: String) throws -> CGRect { try #require(frames[path], "Missing frame at \(path)") }
}

extension LayoutPicture {
    // The 8-bit paint the capture gives a colour that is not a theme token
    // (a system colour, or one composited over a background), or a token an
    // assertion needs bit-exactly. Compare tokens with tolerance against
    // `ThemeColor.sRGB` instead. Geometry is compared in points separately.
    static func swatch(_ color: Color, over background: Color = .clear) throws -> NSColor {
        NativeTestWatchdog.beginWaiting("SwiftUI to render the reference swatch")
        defer { NativeTestWatchdog.finishedWaiting() }
        let reference = color.frame(width: 4, height: 4).background(background)
        return try #require(try capture(reference).sRGBColor(x: 2, y: 2))
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
