import AppKit
import CryptoKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

@Suite(NativeTestWatchdog()) @MainActor struct IconDrawingTests {
    private func lucideResource(_ name: String, extension ext: String) throws -> Data {
        let url = try #require(Bundle.module.url(forResource: name, withExtension: ext, subdirectory: "Lucide"))
        return try Data(contentsOf: url)
    }

    private func sha256(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    @Test func iconDrawsAtAllFourSizesAndDefaultsToMedium() async throws {
        let picture = try await LayoutPicture(#"""
        root = Row([Icon("minus", "sm"), Icon("minus", "md"), Icon("minus", "lg"), Icon("minus", "xl"), Icon("minus")], 0)
        """#)
        for (index, side) in [14, 18, 24, 32, 18].enumerated() {
            let frame = try picture.rect("/children/\(index)")
            #expect(frame.size == CGSize(width: side, height: side))
            // The glyph's horizontal line runs through the measured centre.
            let center = try picture.pixel(Int(frame.midX), Int(frame.midY))
            #expect(center.alphaComponent > 0.35)
            let clear = try picture.pixel(Int(frame.midX), Int(frame.minY + frame.height / 4))
            #expect(clear.alphaComponent <= 1.0 / 255)
            let stroke = try (Int(frame.minY)..<Int(frame.maxY)).reduce(CGFloat.zero) { coverage, y in
                try coverage + picture.pixel(Int(frame.midX), y).alphaComponent
            }
            #expect(abs(stroke - 1.8 * frame.height / 24) < 0.16)
        }
    }

    @Test func iconUsesItsColorOrInheritsTheInlineTextColor() async throws {
        let source = #"""
        root = Col([colors, words, Icon("minus", "xl", "transparent"), smallWords, reference], 0, "start")
        colors = Row([Icon("minus", "xl", "info"), Icon("minus", "xl", "#ff0000"), Icon("minus", "xl", "var(--unused)")], 0)
        words = Text(["Before ", Icon("minus", "xl"), Icon("minus", "xl", "currentColor"), " after"], "md", "#008000")
        smallWords = Text(["Before ", Icon("minus", "sm"), Icon("minus", "sm", "currentColor"), " after"], "md", "#008000")
        reference = Box([], null, 0, 0, "#ff00ff", null, false, 16, 16)
        """#
        for mode: IntelligentUIMode in [.light, .dark] {
            NativeTestWatchdog.waitingFor("Core to load the Intelligent UI library for the \(mode) icon colours")
            let live = try LiveProgram(library: try await Core().intelligentUILibrary())
            NativeTestWatchdog.waitingFor("Core to show the \(mode) icon colours program")
            try await live.show(response: source, streaming: false)
            try #require(live.errors.isEmpty)

            var frames: [String: CGRect] = [:]
            let drawing = ElementDrawing(chrome: { view, _, path in
                AnyView(view.onGeometryChange(for: CGRect.self) { $0.frame(in: .named("iconColors")) } action: {
                    frames[path] = $0
                })
            })
            let size = CGSize(width: 240, height: 160)
            let content = ElementView(live: live).environment(\.elementDrawing, drawing)
                .frame(width: size.width, height: size.height, alignment: .topLeading)
                .coordinateSpace(name: "iconColors").intelligentUIMode(mode).intelligentUIViewport(size)
            NativeTestWatchdog.waitingFor("AppKit to create and mount the \(mode) icon colours window")
            _ = NSApplication.shared
            let host = NSHostingView(rootView: content)
            let window = NSWindow(contentRect: CGRect(origin: CGPoint(x: -20000, y: -20000), size: size),
                                  styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.isOpaque = false
            window.backgroundColor = .clear
            window.colorSpace = .sRGB
            window.contentView = host
            defer { window.closeForTest() }

            NativeTestWatchdog.waitingFor("AppKit to lay out the \(mode) icon colours")
            host.layoutSubtreeIfNeeded()
            let bounds = host.bounds
            NativeTestWatchdog.waitingFor("AppKit to prepare a bitmap of the \(mode) icon colours")
            let bitmap = try #require(host.bitmapImageRepForCachingDisplay(in: bounds))
            NativeTestWatchdog.waitingFor("AppKit to draw the \(mode) icon colours")
            host.cacheDisplay(in: bounds, to: bitmap)
            let scale = CGFloat(bitmap.pixelsWide) / bounds.width

            func frame(_ path: String) throws -> CGRect {
                try #require(frames[path], "Missing frame at \(path) in \(mode)")
            }

            func pixel(_ x: Int, _ y: Int) throws -> NSColor {
                try #require(bitmap.colorAt(x: x, y: y))
            }

            // An opaque Box proves this capture contains a draw, even if an icon is blank.
            let reference = try frame("/children/4")
            let control = try pixel(Int(reference.midX * scale), Int(reference.midY * scale))
            try #require(control.alphaComponent == 1,
                         "Reference Box was not painted in \(mode): \(control)")

            // This icon declares transparent, so its entire frame should be unpainted.
            let transparent = try frame("/children/2")
            let transparentPixels = transparent.applying(CGAffineTransform(scaleX: scale, y: scale)).integral
            let hasPaint = try (Int(transparentPixels.minY)..<Int(transparentPixels.maxY)).contains { y in
                try (Int(transparentPixels.minX)..<Int(transparentPixels.maxX)).contains { x in
                    try pixel(x, y).alphaComponent > 0
                }
            }
            #expect(!hasPaint, "Transparent icon has nonempty painted bounds in \(mode)")
            let transparentCenter = try pixel(Int(transparent.midX * scale), Int(transparent.midY * scale))
            #expect(transparentCenter.alphaComponent == 0, "Transparent icon centre in \(mode)")

            let theme = mode == .light ? IntelligentTheme.light : .dark
            let inlineColor = ThemeColor.hex(0x008000)
            let expected: [(String, ThemeColor)] = [
                ("/children/0/children/0", theme.info),
                ("/children/0/children/1", .hex(0xff0000)),
                ("/children/0/children/2", theme.ink),
                ("/children/1/text/1", inlineColor),
                ("/children/1/text/2", inlineColor),
            ]
            for (path, color) in expected {
                let glyph = try frame(path)
                // Minus runs from (5, 12) to (19, 12) on a 24-point grid.
                // The xl stroke is wide enough to contain an entire pixel at 1x.
                let unit = glyph.width / 24
                let stroke = CGRect(x: glyph.minX + 5 * unit, y: glyph.midY - 0.9 * unit,
                                    width: 14 * unit, height: 1.8 * unit)
                let x = Int(stroke.midX * scale)
                let y = Int(stroke.midY * scale)
                let footprint = CGRect(x: CGFloat(x) / scale, y: CGFloat(y) / scale,
                                       width: 1 / scale, height: 1 / scale)
                try #require(stroke.contains(footprint), "No interior pixel for \(path) in \(mode)")
                let paint = try pixel(x, y)
                // colorAt returns the bitmap's component values with a calibrated RGB tag.
                // Compare those values to the declared colour converted into the bitmap's profile.
                let declared = NSColor(srgbRed: color.red, green: color.green, blue: color.blue, alpha: 1)
                let expectedPaint = try #require(declared.usingColorSpace(bitmap.colorSpace))
                #expect(paint.alphaComponent >= 254.0 / 255, "\(path) in \(mode)")
                #expect(abs(paint.redComponent - expectedPaint.redComponent) < 0.045, "\(path) in \(mode)")
                #expect(abs(paint.greenComponent - expectedPaint.greenComponent) < 0.045, "\(path) in \(mode)")
                #expect(abs(paint.blueComponent - expectedPaint.blueComponent) < 0.045, "\(path) in \(mode)")
            }
            let firstInline = try frame("/children/3/text/1")
            let secondInline = try frame("/children/3/text/2")
            #expect(firstInline.width == 14)
            #expect(firstInline.minX > 0 && firstInline.maxX <= secondInline.minX)
            #expect(firstInline.midY == secondInline.midY)
        }
    }

    @Test(arguments: ["", #", "currentColor""#])
    func iconInAnInlineBlackBoxInheritsWhite(_ colorArgument: String) async throws {
        let picture = try await LayoutPicture(#"""
        root = Col([Text(["Preview ", Box([Icon("minus", "lg"\#(colorArgument))], null, 0, 1, "#000")], "md", "#008000")], 0, "start")
        """#)
        let frame = try picture.rect("/children/0/text/1/children/0")
        let paint = try picture.pixel(Int(frame.midX), Int(frame.midY))
        #expect(paint.redComponent > 0.8 && paint.greenComponent > 0.8 && paint.blueComponent > 0.8)
    }

    @Test func neighboringIconsKeepTheirWidthsInACrowdedRow() async throws {
        let width: CGFloat = 42
        let picture = try await LayoutPicture(#"""
        root = Row([Icon("minus", "xl"), Icon("plus", "sm")], 0)
        """#, width: width)
        let first = try picture.rect("/children/0")
        let second = try picture.rect("/children/1")
        #expect(first.size == CGSize(width: 32, height: 32))
        #expect(second.size == CGSize(width: 14, height: 14))
        #expect(first.maxX == second.minX)
        #expect(first.midY == second.midY)
        for (index, frame) in [first, second].enumerated() {
            let visible = frame.intersection(CGRect(x: 0, y: frame.minY, width: width, height: frame.height))
            #expect(try picture.alpha(in: visible).contains { $0 > 32 }, "Icon \(index)")
        }
    }

    @Test func iconMatchesUpstreamSpellingsAndDrawsFallbackOnlyForUnknownNames() async throws {
        let longSuffix = String(repeating: "A", count: 32_768)
        let cases: [(input: String, key: String)] = [
            ("map-pin", "map-pin"), ("MapPin", "map-pin"), ("mapPin", "map-pin"),
            ("  map_pin  ", "map-pin"), ("map pin", "map-pin"),
            ("ArrowUp10", "arrow-up-1-0"), ("Axis3d", "axis-3d"), ("Grid2x2", "grid-2x2"),
            ("home", "house"), ("file-x-2", "file-x-corner"),
            ("arrow-down-up", "arrow-down-up"), ("ArrowUpDown", "arrow-up-down"),
            ("move-right-up", "move-up-right"), ("MoveRightUp", "move-up-right"),
            ("MapPinDetailedMore", "map-pin"), ("pin-map-extra", "map-pin"),
            ("map-pin-\u{0301}", "map-pin"),
            ("map-pin-\(longSuffix)", "map-pin"),
            ("unknown-glyph-name", "circle-dot"),
        ]
        for (input, key) in cases {
            let picture = try await LayoutPicture(#"root = Row([Icon("\#(input)", "lg"), Icon("\#(key)", "lg")], 0)"#)
            let inputAlpha = try picture.alpha(in: picture.rect("/children/0"))
            let keyAlpha = try picture.alpha(in: picture.rect("/children/1"))
            #expect(keyAlpha.contains { $0 > 32 }, "\(key)")
            #expect(!zip(inputAlpha, keyAlpha).contains { abs(Int($0) - Int($1)) > 1 },
                    "\(input.prefix(80)) should draw \(key)")
        }

        let picture = try await LayoutPicture(#"""
        root = Row([Icon("", "lg"), Icon("   ", "lg"), Icon("circle-dot", "lg"), Icon("circle", "lg"), Icon("arrow-up-down", "lg"), Icon("arrow-down-up", "lg")], 0)
        """#)
        for index in [0, 1] {
            let frame = try picture.rect("/children/\(index)")
            #expect(frame.size == CGSize(width: 24, height: 24))
            #expect(try picture.alpha(in: frame).allSatisfy { $0 <= 1 })
        }
        let circleDot = try picture.alpha(in: picture.rect("/children/2"))
        let circle = try picture.alpha(in: picture.rect("/children/3"))
        #expect(zip(circleDot, circle).contains { abs(Int($0) - Int($1)) > 32 })
        let upDown = try picture.alpha(in: picture.rect("/children/4"))
        let downUp = try picture.alpha(in: picture.rect("/children/5"))
        #expect(zip(upDown, downUp).contains { abs(Int($0) - Int($1)) > 32 })
    }

    /// The 1,912 icons draw in 16 batches of 120. One case drawing all of them
    /// took 48.6–59.5 s on GitHub's runner (runs 46001981058, 46021355831,
    /// 46035743129) and hit the 60 s watchdog in run 46040620093, so the
    /// batches are split across four cases of four batches each. Every icon is
    /// still drawn and compared exactly once.
    nonisolated static let iconShards = 4

    @Test(arguments: 0..<iconShards) func everyPinnedIconNameDrawsItsArtwork(shard: Int) async throws {
        struct Catalog: Decodable {
            let names: [String: String]
            let artwork: [String: String]
        }
        let catalog = try JSONDecoder().decode(Catalog.self, from: lucideResource("catalog", extension: "json"))
        let names = catalog.names.keys.sorted()
        // Digest of the sorted wire names in lucide-react 0.562.0's dynamicIconImports.
        #expect(names.count == 1912)
        #expect(sha256(Data(names.joined(separator: "\n").utf8)) ==
                "d89903f97f65a5b0c11177e462690ac5d6ab86dfd08db2d778ee35e8cfe6c535")

        // Decode the expected geometry directly by its pinned key, bypassing the renderer's lookup and cache.
        let expectedByKey = try catalog.artwork.mapValues { markup in
            try #require(LucideShape.read(markup))
        }
        // Render in batches so the picture and the program stay small.
        let columns = 20
        let rowsPerBatch = 6
        let batchStarts = Array(stride(from: 0, to: names.count, by: columns * rowsPerBatch))
        for (batchIndex, start) in batchStarts.enumerated() where batchIndex % Self.iconShards == shard {
            let batch = Array(names.dropFirst(start).prefix(columns * rowsPerBatch))
            let rows = stride(from: 0, to: batch.count, by: columns).map { offset in
                let icons = batch.dropFirst(offset).prefix(columns).map { #"Icon("\#($0)", "lg")"# }
                return "Row([\(icons.joined(separator: ","))], 0)"
            }
            let source = #"root = Col([\#(rows.joined(separator: ","))], 0, "start")"#
            let width = CGFloat(columns * 24), height = CGFloat(rowsPerBatch * 24)
            let picture = try await LayoutPicture(source, width: width, height: height)
            let reference = try await LayoutPicture(source, width: width, height: height, component: { element, _ in
                guard element.typeName == "Icon", let name = element.word("name"),
                      let key = catalog.names[name], let shapes = expectedByKey[key] else { return nil }
                return AnyView(Canvas { context, _ in
                    let stroke = StrokeStyle(lineWidth: 1.8, lineCap: .round, lineJoin: .round)
                    for shape in shapes {
                        if shape.filled { context.fill(shape.path, with: .color(.black)) }
                        context.stroke(shape.path, with: .color(.black), style: stroke)
                    }
                }.frame(width: 24, height: 24))
            })
            for (index, name) in batch.enumerated() {
                let path = "/children/\(index / columns)/children/\(index % columns)"
                let frame = try picture.rect(path)
                #expect(frame.size == CGSize(width: 24, height: 24), "\(name)")
                let alpha = try picture.alpha(in: frame)
                #expect(alpha.contains { $0 > 32 }, "\(name)")
                let key = try #require(catalog.names[name])
                try #require(expectedByKey[key] != nil, "\(name) → \(key)")
                let expected = try reference.alpha(in: reference.rect(path))
                #expect(alpha.count == expected.count &&
                        !zip(alpha, expected).contains { abs(Int($0) - Int($1)) > 1 }, "\(name) → \(key)")
            }
        }
    }

    @Test func theLucideArtworkLicenseShipsInTheSwiftPackage() throws {
        // The release notice includes both the ISC grant and the notice for Feather portions.
        #expect(try sha256(lucideResource("LICENSE", extension: "txt")) ==
                "3c57143f22dac950efebf84002e360bba5458c7b90fe583b0667aaaef1a7256e")
    }
}
