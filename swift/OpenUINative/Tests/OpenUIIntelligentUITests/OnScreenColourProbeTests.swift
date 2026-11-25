import AppKit
import OpenUIIntelligentUI
import SwiftUI
import Testing

extension CollectionDrawingTests {
    @Suite @MainActor struct OnScreenColourProbeTests {
        @Test func windowColoursConvertToDeclaredSRGB() async throws {
            let probe = RunnerDisplayProbe.screenColour
            try await probe.run { try await measureWindowColours(prefix: probe.prefix) }
        }

        private func measureWindowColours(prefix: String) async throws {
            let session = RunnerDisplaySession(prefix: prefix)
            defer { session.restore() }
            let fixture = try await CollectionWindow("""
            root = Col([green, token], 12)
            green = Box([], null, 0, 0, "#00ff00", null, false, 80, 50)
            token = Box([], null, 0, 0, "chart2", null, false, 80, 50)
            """, width: 200, height: 150)
            defer { fixture.window.close() }
            let host = fixture.host
            host.rootView = AnyView(host.rootView.intelligentUIMode(.light))
            try await session.present(fixture)
            logDisplayProbe("\(prefix) cacheDisplay; \(session.describe(fixture.window))")

            let bounds = host.bounds
            let bitmap = try #require(host.bitmapImageRepForCachingDisplay(in: bounds),
                                      "\(prefix) Could not allocate a bitmap for \(bounds)")
            host.cacheDisplay(in: bounds, to: bitmap)
            let source = try #require(bitmap.cgImage?.colorSpace, "\(prefix) Cached image has no colour space")
            let sRGB = try #require(CGColorSpace(name: CGColorSpace.sRGB))
            let spaceName = source.name.map { $0 as String } ?? bitmap.colorSpace.localizedName ?? "(unnamed)"
            logDisplayProbe("\(prefix) image colour space=\(spaceName); bitmap space=\(bitmap.colorSpace.localizedName ?? "(unnamed)"); "
                + "\(bitmap.pixelsWide)x\(bitmap.pixelsHigh) px, \(bitmap.bitsPerSample) bits/sample")
            try #require(source.model == .rgb && bitmap.bitsPerSample == 8
                         && bitmap.samplesPerPixel == (bitmap.hasAlpha ? 4 : 3),
                         "\(prefix) Unsupported cache pixel format for \(spaceName)")

            let swatches: [(label: String, declared: ThemeColor)] = [
                ("#00ff00", ThemeColor(red: 0, green: 1, blue: 0)),
                ("chart2 (light)", IntelligentTheme.light.chart2),
            ]
            for (index, swatch) in swatches.enumerated() {
                // CollectionWindow measures SwiftUI frames from the top-left.
                let frame = try fixture.rect("/children/\(index)")
                let x = Int(frame.midX * CGFloat(bitmap.pixelsWide) / bounds.width)
                let y = Int(frame.midY * CGFloat(bitmap.pixelsHigh) / bounds.height)
                try #require((0..<bitmap.pixelsWide).contains(x) && (0..<bitmap.pixelsHigh).contains(y),
                             "\(prefix) \(swatch.label) centre is outside the cached image: (\(x), \(y))")
                var samples = Array(repeating: 0, count: bitmap.samplesPerPixel)
                bitmap.getPixel(&samples, atX: x, y: y)
                let alphaFirst = bitmap.hasAlpha && bitmap.bitmapFormat.contains(.alphaFirst)
                let rgb = (alphaFirst ? samples.dropFirst() : samples.prefix(3)).map { CGFloat($0) / 255 }
                let alpha = bitmap.hasAlpha ? (alphaFirst ? samples[0] : samples[3]) : 255
                let declared = [swatch.declared.red, swatch.declared.green, swatch.declared.blue].map { CGFloat($0) }
                let context = "\(prefix) \(swatch.label) at (\(x), \(y)) in \(spaceName); raw=\(samples) "
                    + "normalized RGB=\(rgb) declared sRGB=\(declared)"
                try #require(alpha == 255, "\(context); expected an opaque swatch, alpha byte=\(alpha)")
                let colour = try #require(CGColor(colorSpace: source, components: rgb + [1]), "\(context); invalid pixel")
                let converted = try #require(colour.converted(to: sRGB, intent: .relativeColorimetric, options: nil)?.components,
                                             "\(context); Core Graphics sRGB conversion failed")
                let actual = Array(converted.prefix(3))
                let deltas = zip(actual, declared).map { abs($0 - $1) }
                let matches = actual.count == 3 && deltas.allSatisfy { $0 <= 2.0 / 255 }
                let result = "\(context); converted sRGB=\(actual) absolute deltas=\(deltas)"
                logDisplayProbe("\(result); \(matches ? "PASS" : "FAIL (outside 2/255)")")
                #expect(matches, "\(result); colour-managed window capture differs from the declared sRGB colour")
            }
        }
    }
}
