import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import Testing

@Suite @MainActor struct LayoutDrawingTests {
    @Test func mixedLayoutDrawsPanelsRuleAndFreeSpaceWithoutPlaceholders() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Card([Row([tile, Spacer(), tile])], "outline", 0, 2), Divider()], 1)
        tile = Box([], "col", 0, 0, "#ff0000", null, false, 20, 10)
        """)
        #expect(picture.fallback.isEmpty)
        #expect(try picture.rect("/children/0") == CGRect(x: 0, y: 0, width: 240, height: 28))
        let row = "/children/0/children/0"
        #expect(try picture.rect(row + "/children/0") == CGRect(x: 9, y: 9, width: 20, height: 10))
        #expect(try picture.rect(row + "/children/2").maxX == 231)
        #expect(try picture.rect("/children/1") == CGRect(x: 0, y: 32, width: 240, height: 1))
    }
}

extension LayoutDrawingTests {
    @Test func flowUsesSiblingAndAncestorSpacingAndExplicitGapWins() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Divider(), Divider(), Caption("a"), Title("b"), Text("c"), Text("d"), Title("e"), Row([Badge("f")]), Title("g"), nested, explicit])
        nested = Box([Divider(), Divider()])
        explicit = Col([Divider(), Divider()], 0)
        """, height: 500)
        let root = "/children/"
        func distance(_ a: String, _ b: String) throws -> CGFloat {
            try picture.rect(b).minY - picture.rect(a).maxY
        }
        for (i, expected) in [24, 24, 4, 8, 12, 24, 8, 24, 12, 24].enumerated() {
            #expect(try distance(root + String(i), root + String(i + 1)) == CGFloat(expected))
        }
        #expect(try distance(root + "9/children/0", root + "9/children/1") == 12)
        #expect(try distance(root + "10/children/0", root + "10/children/1") == 0)
    }

    @Test func rowsAlignJustifyAndWrapInSourceOrder() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Row([a, b]), Row([a, b], 1, "end", "between"), Row([a, b, a], 1, "start", "center", true)], 0)
        a = Box([], null, null, null, "red", null, null, 42, 10)
        b = Box([], null, null, null, "blue", null, null, 40, 20)
        """, width: 100)
        func frame(_ row: Int, _ i: Int) throws -> CGRect { try picture.rect("/children/\(row)/children/\(i)") }
        #expect(try frame(0, 0).origin == CGPoint(x: 0, y: 5))
        #expect(try frame(0, 1).minX == 50)
        #expect(try frame(1, 0).minY == 30)
        #expect(try frame(1, 1).maxX == 100)
        #expect(try frame(2, 0).origin == CGPoint(x: 7, y: 40))
        #expect(try frame(2, 2).origin == CGPoint(x: 29, y: 64))
    }

    @Test func boxResolvesLengthsFlexAndColumnWrappingInsideItsBorderBox() async throws {
        let picture = try await LayoutPicture("""
        root = Col([panel], 0, "end")
        panel = Box([tile, tile, tile], "col", 1, 1, "canvas", "md", true, "50%", 50, null, "center", "end", true)
        tile = Box([], null, null, null, "red", null, null, "1em", 18)
        """, width: 240)
        #expect(try picture.rect("/children/0") == CGRect(x: 120, y: 0, width: 120, height: 50))
        let path = "/children/0/children/"
        #expect(try picture.rect(path + "0") == CGRect(x: 143.5, y: 5, width: 16, height: 18))
        #expect(try picture.rect(path + "2") == CGRect(x: 200.5, y: 27, width: 16, height: 18))
        // In a stretched Row, a flex basis of zero overrides the given width.
        let flex = try await LayoutPicture("""
        root = Col([Row([fixed, one, two], 0)])
        fixed = Box([], null, null, null, null, null, null, 60, 10)
        one = Box([], null, null, null, null, null, null, 200, 10, 1)
        two = Box([], null, null, null, null, null, null, null, 10, 2)
        """, width: 240)
        #expect(try flex.rect("/children/0/children/1").width == 60)
        #expect(try flex.rect("/children/0/children/2").width == 120)
    }

    @Test func aDirectPhotoChangesRowRhythmAndABoxedPhotoOnlyChangesSize() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Row([photo, block], 0, "end"), Row([Box([photo]), block], 0, "end")], 0)
        photo = Image("", "Photo")
        block = Box([], null, null, null, null, null, null, 20, 10)
        """, width: 400, height: 260, component: { e, _ in
            e.typeName == "Image" ? AnyView(Color.blue) : nil
        })
        let direct = "/children/0/children/", boxed = "/children/1/children/"
        #expect(try picture.rect(direct + "0").size == CGSize(width: 132, height: 99))
        #expect(try picture.rect(direct + "1").origin == CGPoint(x: 148, y: 0))
        #expect(try picture.rect(boxed + "0/children/0").size == CGSize(width: 132, height: 99))
        #expect(try picture.rect(boxed + "1").origin == CGPoint(x: 132, y: 188))
    }

    @Test func cardPaddingFollowsViewportAndNestingAndExplicitPaddingWins() async throws {
        let source = """
        root = Col([Card([Card([Divider()]), Card([Divider()], "soft", 0, 1)])])
        """
        for (width, outer) in [(640, 17), (641, 21)] {
            let picture = try await LayoutPicture(source, width: CGFloat(width))
            let path = "/children/0/children/"
            #expect(try picture.rect(path + "0").minX == CGFloat(outer))
            #expect(try picture.rect(path + "0/children/0").minX == CGFloat(outer + 17))
            #expect(try picture.rect(path + "1/children/0").minX == CGFloat(outer + 5))
            #expect(try picture.rect(path + "1").minY - picture.rect(path + "0").maxY == 12)
        }
    }
}

extension LayoutDrawingTests {
    @Test func cardVariantsAndBoxRadiusBorderAndPaddingPaintInBothModes() async throws {
        for mode in [IntelligentUIMode.light, .dark] {
            let picture = try await LayoutPicture("""
            root = Col([Card([], "outline"), Card([], "soft"), Box([tile], null, 0, 1, "#ff0000", "md", true, 40, 40)], 0)
            tile = Box([], null, null, null, "#0000ff", null, null, 50, 50)
            """, width: 80, height: 150, mode: mode)
            let theme: IntelligentTheme = mode == .light ? .light : .dark
            let outline = try picture.pixel(40, 12), soft = try picture.pixel(40, 46)
            #expect(try outline == LayoutPicture.swatch(theme.canvas.color))
            #expect(outline.alphaComponent == 1)
            #expect(abs(soft.alphaComponent - theme.surface.alpha) < 0.01)
            let box = try picture.rect("/children/2")
            let y = Int(box.minY)
            #expect(try picture.pixel(0, y).alphaComponent == 0)
            #expect(try picture.pixel(20, y + 2).redComponent > 0.99)
            #expect(try picture.pixel(20, y).greenComponent > picture.pixel(20, y + 2).greenComponent)
            #expect(try picture.pixel(20, y + 10).blueComponent > 0.99)
            #expect(try picture.pixel(41, y + 15).alphaComponent == 0)
        }
    }

    @Test func dividerUsesSeparatorsBorderAndOnColorReachesDescendantTokens() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Divider(), Box([tile], null, 0, 0, "#000"), tile], 0)
        tile = Box([], null, null, null, "muted", null, null, 20, 10)
        """)
        let separator = try picture.pixel(100, 0)
        #expect(abs(separator.alphaComponent - 0.06) < 0.005)
        let inside = try picture.pixel(5, 5)
        #expect(try inside == LayoutPicture.swatch(ThemeColor.hex(0xffffff).mixedWithTransparent(0.72).color, over: .black))
        #expect(inside.alphaComponent == 1)
        #expect(abs(try picture.pixel(5, 15).alphaComponent - IntelligentTheme.light.muted.alpha) < 0.01)
    }

    @Test func relativeLengthsUseTheViewportFontAndContainingBlock() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Card([Box([tile, rem, percent], "row", 0, 0, null, null, false, "50%", 40)], "outline", 0, 0)])
        tile = Box([], null, null, null, null, null, false, "10vw", "10vh")
        rem = Box([], null, null, null, null, null, false, "2rem", "1em")
        percent = Box([], null, null, null, null, null, false, "20%", "50%")
        """, width: 402, height: 200)
        let path = "/children/0/children/0/children/"
        #expect(try abs(picture.rect(path + "0").width - 40.2) < 0.01)
        #expect(try picture.rect(path + "0").height == 20)
        #expect(try picture.rect(path + "1").size == CGSize(width: 32, height: 14))
        #expect(try picture.rect(path + "2").size == CGSize(width: 40, height: 20))
    }

    @Test func cardOpeningTitleSubtitleAndLargeTypeUseCardMetrics() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Card([Title("First"), Caption("Next"), Title("Big", "lg"), Text("Body")], "outline", 1, 0), Card([Row([Title("Row title")])])], 0)
        """)
        let path = "/children/0/children/"
        for (i, expected) in [20, 18, 24, 22].enumerated() {
            #expect(try picture.rect(path + String(i)).height == CGFloat(expected))
        }
        #expect(try picture.rect(path + "1").minY - picture.rect(path + "0").maxY == 6)
        #expect(try picture.rect("/children/1/children/0/children/0").height == 20)
    }
}

// Intercept network requests before DNS. A file URL for a preloaded pipe
// also detects direct reads that bypass the URL loading system.
private final class BackgroundRequest: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "layout-background-test.invalid"
    }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Issue.record("Box background attempted a network request")
        client?.urlProtocol(self, didFailWithError: URLError(.unsupportedURL))
    }
    override func stopLoading() {}
}

extension LayoutDrawingTests {
    @Test func backgroundPaintConsumesNoPipeBytesAndMakesNoInterceptedRequest() async throws {
        let pipe = Pipe()
        let sentinel = Data("unused".utf8)
        try pipe.fileHandleForWriting.write(contentsOf: sentinel)
        try pipe.fileHandleForWriting.close()
        let file = URL(fileURLWithPath: "/dev/fd/\(pipe.fileHandleForReading.fileDescriptor)")
        try #require(URLProtocol.registerClass(BackgroundRequest.self))
        defer { URLProtocol.unregisterClass(BackgroundRequest.self) }
        let library = try await Core().intelligentUILibrary()
        for value in ["url(\(file.absoluteString))", "url(https://layout-background-test.invalid/photo)",
                      "image-set(url(https://layout-background-test.invalid/photo) 1x)",
                      "linear-gradient(red, blue)", "not-a-color"] {
            try await expectTransparentBackground(value, library: library)
        }
        // A mounted view also exercises any appearance-triggered loader.
        let live = try LiveProgram(library: library)
        try await live.show(response: """
        root = Col([Box([], null, 0, 0, "url(\(file.absoluteString))", null, false, 32, 32), Box([], null, 0, 0, "url(https://layout-background-test.invalid/photo)", null, false, 32, 32)])
        """, streaming: false)
        let host = NSHostingView(rootView: ElementView(live: live))
        host.setFrameSize(CGSize(width: 100, height: 100))
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(50))
        host.layoutSubtreeIfNeeded()
        // Opening the same file URL and reading to EOF now consumes the
        // sentinel. Any previous reader would have taken those bytes.
        let reader = try FileHandle(forReadingFrom: file)
        defer { try? reader.close() }
        #expect(try reader.readToEnd() == sentinel)
    }

    private func expectTransparentBackground(_ value: String, library: Library) async throws {
        let live = try LiveProgram(library: library)
        try await live.show(response: """
        root = Col([subject, control], 0)
        subject = Box([], "col", 0, 0, "\(value)", null, false, 32, 32)
        control = Box([], "col", 0, 0, "#ff0000", null, false, 32, 32)
        """, streaming: false)
        #expect(live.errors.map(\.message) == [])

        // Draw into a cleared bitmap at one pixel per point.
        let bitmap = try #require(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: 32, pixelsHigh: 64, bitsPerSample: 8, samplesPerPixel: 4,
            hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        ))
        let context = try #require(NSGraphicsContext(bitmapImageRep: bitmap)).cgContext
        let size = bitmap.size
        let view = ElementView(live: live).intelligentUIMode(.light).intelligentUIViewport(size)
            .frame(width: size.width, height: size.height, alignment: .topLeading)
        context.clear(CGRect(origin: .zero, size: size))
        ImageRenderer(content: view).render(rasterizationScale: 1) { _, draw in draw(context) }

        // The second Box verifies the same draw painted a supported background.
        // Both samples are at the centers of the boxes, away from shape edges.
        let painted = try #require(bitmap.colorAt(x: 16, y: 48))
        let center = try #require(bitmap.colorAt(x: 16, y: 16))
        #expect(painted.alphaComponent == 1, "\(value), painted Box: \(painted)")
        #expect(center.alphaComponent == 0, "\(value), tested Box: \(center)")
    }
}

extension LayoutDrawingTests {
    @Test func viewportTracksTheMountedScrollViewThroughAResize() async throws {
        let live = try LiveProgram(library: try await Core().intelligentUILibrary())
        try await live.show(response: """
        root = Col([Box([Card([tile])], null, 0, 0, null, null, false, 180)])
        tile = Box([], null, null, null, null, null, false, "10vw", "10vh")
        """, streaming: false)
        var tileFrame: CGRect?
        let drawing = ElementDrawing(chrome: { view, e, _ in
            guard e.statementId == "tile" else { return view }
            return AnyView(view.onGeometryChange(for: CGRect.self, of: { $0.frame(in: .named("scroll")) }) {
                tileFrame = $0
            })
        })
        let content = ScrollView { ElementView(live: live) }
            .environment(\.elementDrawing, drawing).coordinateSpace(name: "scroll")
        let host = NSHostingView(rootView: content)
        host.sizingOptions = []
        for (size, expectedInset) in [(CGSize(width: 400, height: 200), 17), (CGSize(width: 700, height: 300), 21)] {
            host.setFrameSize(size)
            host.layoutSubtreeIfNeeded()
            await layoutOnMainQueue(host)
            let frame = try #require(tileFrame)
            #expect(frame.size == CGSize(width: size.width / 10, height: size.height / 10))
            #expect(frame.minX == CGFloat(expectedInset))
        }
    }
}

extension LayoutDrawingTests {
    @Test func flexRespectsBorderBoxMinimumAndKeepsNumericWidthsWhenCrowded() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Row([padded, plain], 0), Row([fixed, flexible], 0), Box([padded, plain], "row", 0, 0, null, null, false, 20)])
        padded = Box([], null, 0, 2, null, null, true, null, 20, 1)
        plain = Box([], null, 0, 0, null, null, false, null, 20, 1)
        fixed = Box([], null, 0, 0, null, null, false, 80, 10)
        flexible = Box([], null, 0, 0, null, null, false, "80px", 10)
        """, width: 100)
        #expect(try picture.rect("/children/0/children/0").width == 50)
        #expect(try picture.rect("/children/0/children/1").width == 50)
        #expect(try picture.rect("/children/1/children/0").width == 80)
        #expect(try picture.rect("/children/1/children/1").width == 20)
        #expect(try picture.rect("/children/2/children/1").minX == 18)
        #expect(try picture.rect("/children/2/children/1").width == 2)
    }

    @Test func columnAlignmentPositionsChildrenAndAlignsTheirText() async throws {
        struct Probe: View {
            @Environment(\.multilineTextAlignment) var alignment
            var body: some View {
                (alignment == .center ? Color.red : alignment == .trailing ? Color.blue : Color.green)
                    .frame(width: 20, height: 6)
            }
        }
        let picture = try await LayoutPicture("""
        root = Col([Col([Label("A")], 0, "center"), Col([Label("B")], 0, "end"), Col([Label("C")], 0, "start")], 0)
        """, width: 100, component: { e, _ in e.typeName == "Label" ? AnyView(Probe()) : nil })
        #expect(try picture.rect("/children/0/children/0").minX == 40)
        #expect(try picture.rect("/children/1/children/0").minX == 80)
        #expect(try picture.rect("/children/2/children/0").minX == 0)
        #expect(try picture.pixel(50, 2) == LayoutPicture.swatch(.red))
        #expect(try picture.pixel(90, 8) == LayoutPicture.swatch(.blue))
        #expect(try picture.pixel(10, 14) == LayoutPicture.swatch(.green))
    }
}

extension LayoutDrawingTests {
    @Test func centeredColumnMeasuresWrappedTextAtItsAvailableWidth() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Text("Paragraph"), Divider()], 0, "center")
        """, width: 80, component: { e, _ in
            e.typeName == "Text" ? AnyView(Text(verbatim: "Several words on several lines").font(.system(size: 16))) : nil
        })
        let text = try picture.rect("/children/0")
        #expect(text.height > 35)
        #expect(try picture.rect("/children/1").minY == text.maxY)
    }
}

extension LayoutDrawingTests {
    @Test func rootColSuppliesThemeInkAndFontDesign() async throws {
        let live = try LiveProgram(library: try await Core().intelligentUILibrary())
        try await live.show(response: #"root = Col([Text("iiiiiiii")])"#, streaming: false)
        let text = Text(verbatim: "iiiiiiii")
        let drawing = ElementDrawing(component: { e, _ in e.typeName == "Text" ? AnyView(text) : nil })
        let view = ElementView(live: live).environment(\.elementDrawing, drawing)
            .font(.system(size: 20)).fontDesign(.monospaced).foregroundStyle(.red)
            .intelligentUIMode(.dark)
        let reference = text.font(.system(size: 20)).fontDesign(.default)
            .foregroundStyle(IntelligentTheme.dark.ink.color).intelligentUIMode(.dark)
        let actual = try #require(ImageRenderer(content: view).cgImage)
        let expected = try #require(ImageRenderer(content: reference).cgImage)
        #expect(actual.width == expected.width)
        #expect(actual.dataProvider?.data as Data? == expected.dataProvider?.data as Data?)
    }
}

@Suite @MainActor struct LayoutReviewTests {
    @Test func eachKeepsNestedIndicesAndDividerPosition() async throws {
        let picture = try await LayoutPicture("""
        root = Col([@Each([1,2], "i", Box([],null,0,0,"red",null,null,10,10)), Divider()], 0)
        """, width: 100)
        #expect(try picture.rect("/children/1") == CGRect(x: 0, y: 20, width: 100, height: 1))
        for i in 0..<2 {
            #expect(try picture.rect("/children/0/\(i)") == CGRect(x: 0, y: 10 * i, width: 10, height: 10))
        }
    }

    @Test func adjacentPrimitiveChildrenDrawAsText() async throws {
        let picture = try await LayoutPicture("""
        root = Col([["A", [42, true, null], {unused: 1}], Divider(), Text("A42true"), Divider()], 0)
        """, width: 100, component: { e, _ in
            e.typeName == "Text" ? AnyView(Text(verbatim: "A42true")) : nil
        })
        let line = Int(try picture.rect("/children/3").minY - picture.rect("/children/1").maxY)
        try #require(line > 0)
        try #require(picture.rect("/children/1").minY == CGFloat(line))
        for y in 0..<line {
            for x in 0..<60 {
                #expect(try picture.pixel(x, y) == picture.pixel(x, line + 1 + y))
            }
        }
    }

    @Test func columnGrowFreezesAtContentMinimum() async throws {
        for (height, first, second) in [(20, 40, 0), (60, 40, 20), (100, 50, 50)] {
            let picture = try await LayoutPicture("""
            root = Col([Box([first, second], "col", 0, 0, null, null, null, 100, \(height))], 0)
            first = Box([Box([], null, 0, 0, "red", null, null, 10, 40)], null, 0, 0, null, null, null, null, null, 1)
            second = Box([], null, 0, 0, null, null, null, null, null, 1)
            """)
            #expect(try picture.rect("/children/0/children/0").height == CGFloat(first))
            #expect(try picture.rect("/children/0/children/1") == CGRect(x: 0, y: first, width: 100, height: second))
        }
    }

    @Test func columnShrinkFreezesAtCappedContentMinimum() async throws {
        for (specified, expected) in [("40", 40), ("16", 16)] {
            let picture = try await LayoutPicture("""
            root = Col([Box([first, second], "col", 0, 0, null, null, null, 100, 30)], 0)
            first = Box([Box([], null, 0, 0, "red", null, null, 10, 40)], null, 0, 0, null, null, null, null, \(specified))
            second = Box([], null, 0, 0, null, null, null, null, 20)
            """)
            #expect(try picture.rect("/children/0/children/0").height == CGFloat(expected))
            #expect(try picture.rect("/children/0/children/1") == CGRect(x: 0, y: expected, width: 100, height: max(0, 30 - expected)))
        }
    }

    @Test func percentageInAutoHeightRowAndColumnUsesAuto() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Row([tall, half], 0), Col([tall, half], 0), Divider()], 0)
        tall = Box([], null, 0, 0, null, null, null, 10, 80)
        half = Box([], null, 0, 0, "red", null, null, 10, "50%")
        """, height: 220)
        #expect(try picture.rect("/children/0/children/1") == CGRect(x: 10, y: 40, width: 10, height: 0))
        #expect(try picture.rect("/children/1/children/1") == CGRect(x: 0, y: 160, width: 10, height: 0))
        #expect(try picture.rect("/children/2").minY == 160)
    }

    @Test func elementShapedColForwardsOnlyColProps() async throws {
        let picture = try await LayoutPicture("""
        root = Col([{type:"element",typeName:"Col",props:{children:[Divider()],padding:4,border:true},partial:false,hasDynamicProps:false}],0)
        """, width: 100)
        let frame = CGRect(x: 0, y: 0, width: 100, height: 1)
        #expect(try picture.rect("/children/0") == frame)
        #expect(try picture.rect("/children/0/children/0") == frame)
    }

    @Test func emUsesHostAndCardFontSizes() async throws {
        for size: CGFloat in [16, 24] {
            let picture = try await LayoutPicture("""
            root = Col([tile, Card([tile])], 0)
            tile = Box([], null, 0, 0, "red", null, null, "2em", "1em")
            """, font: .system(size: size))
            #expect(try picture.rect("/children/0").size == CGSize(width: 2 * size, height: size))
            #expect(try picture.rect("/children/1/children/0").size == CGSize(width: 28, height: 14))
        }
    }

    @Test func exportUsesViewportOnFirstDraw() async throws {
        let live = try LiveProgram(library: try await Core().intelligentUILibrary())
        try await live.show(response: """
        root = Col([Box([tile], null, 0, 0, null, null, null, 100)])
        tile = Box([], null, 0, 0, "red", null, null, "10vw", "10vh")
        """, streaming: false)
        let size = CGSize(width: 400, height: 300)
        let view = ElementView(live: live)
            .intelligentUIViewport(size)
            .frame(width: size.width, height: size.height, alignment: .topLeading)
        let bitmap = NSBitmapImageRep(cgImage: try #require(ImageRenderer(content: view).cgImage))
        for (x, y) in [(5, 5), (39, 29), (40, 29), (39, 30)] {
            let pixel = try #require(bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB))
            if x < 40 && y < 30 {
                #expect(try pixel == LayoutPicture.swatch(ThemeColor.hex(0xff0000).color))
            } else {
                #expect(pixel.alphaComponent == 0)
            }
        }
    }

    @Test func rejectedFillStillSetsDescendantInk() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Box([Box([], null, 0, 0, "ink", null, null, 10, 10)], null, 0, 0, "rgb(0 0 0) var(--bad)", null, null, 20, 20)])
        """)
        #expect(try picture.pixel(15, 15).alphaComponent == 0)
        #expect(try picture.pixel(5, 5) == LayoutPicture.swatch(.white))
    }

    @Test func extremeFlexFactorsKeepBothSiblingsVisible() async throws {
        for (a, b, first, second) in [("1e80", "1e40", 50, 50), ("1e308", "1e307", 50, 50),
                                     ("6", "2", 75, 25), ("0.25", "0.5", 25, 50)] {
            let picture = try await LayoutPicture("""
            root = Col([Row([Box([],null,0,0,"red",null,null,null,10,\(a)), Box([],null,0,0,"blue",null,null,null,10,\(b))], 0)])
            """, width: 100)
            #expect(try picture.rect("/children/0/children/0").width == CGFloat(first))
            #expect(try picture.rect("/children/0/children/1").width == CGFloat(second))
        }
    }

    @Test func commentsSeparateColorTokens() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Box([],null,0,0,"rgb(255 /* red */ 0 0)",null,null,10,10), Box([],null,0,0,"r/**/ed",null,null,10,10)], 0)
        """)
        #expect(try picture.pixel(5, 5) == LayoutPicture.swatch(ThemeColor.hex(0xff0000).color))
        #expect(try picture.pixel(5, 15).alphaComponent == 0)
    }
}

@Suite @MainActor struct LayoutSizingAndInkTests {
    @Test func flexBoxInAutoHeightColumnUsesContentHeight() async throws {
        for (flex, height) in [("1", 12), ("0", 12), ("null", 64)] {
            let picture = try await LayoutPicture("""
            root = Col([Col([panel, last], 0, "end")], 0)
            panel = Box([tile], null, 0, null, null, null, null, null, 64, \(flex))
            tile = Box([], null, 0, null, "red", null, null, 12, 12)
            last = Box([], null, 0, null, "red", null, null, 10, 8)
            """, width: 100)
            #expect(try picture.rect("/children/0/children/0").height == CGFloat(height))
            #expect(try picture.rect("/children/0/children/1").minY == CGFloat(height))
        }

        // The aligned panel has 100 points for two 60-point tiles. Measuring
        // its content at an unconstrained width would miss the second line.
        let wrapped = try await LayoutPicture("""
        root = Col([Col([panel, Divider()], 0, "end")], 0)
        panel = Box([tile, tile], "row", 0, null, null, null, null, null, 64, 1, null, null, true)
        tile = Box([], null, 0, null, "red", null, null, 60, 12)
        """, width: 100)
        #expect(try wrapped.rect("/children/0/children/1").minY == 24)
    }

    @Test func percentageTilesShareALineWithinOnePoint() async throws {
        for (width, fits) in [("33.34%", true), ("33.6%", true), ("34%", false), ("33.6px", false)] {
            let picture = try await LayoutPicture("""
            root = Col([Row([a, b, c], 0, "end", null, true)], 0)
            a = Box([], null, 0, null, "red", null, null, "\(width)", 10)
            b = Box([], null, 0, null, "red", null, null, "\(width)", 20)
            c = Box([], null, 0, null, "red", null, null, "\(width)", 8)
            """, width: 100)
            let second = try picture.rect("/children/0/children/1")
            let third = try picture.rect("/children/0/children/2")
            #expect(try picture.rect("/children/0").height == (fits ? 20 : 28))
            #expect(third.minY == (fits ? 12 : 20))
            #expect(abs(third.minX - (fits ? second.maxX : 0)) < 0.01)
        }
    }

    @Test func dividersShrinkToHalfAPointBesideABox() async throws {
        let rules = Array(repeating: "Divider()", count: 8).joined(separator: ", ")
        let picture = try await LayoutPicture("""
        root = Col([Box([\(rules), blue], null, 0, 0, null, null, null, 100, 8),
                    Box([Divider(), Divider()], null, 0, 0, null, null, null, 100, 1)], 0)
        blue = Box([], null, 0, 0, "blue", null, null, null, 8)
        """, width: 100)
        for (box, count, top) in [(0, 8, 0), (1, 2, 8)] {
            for i in 0..<count {
                let frame = try picture.rect("/children/\(box)/children/\(i)")
                #expect(frame == CGRect(x: 0, y: CGFloat(top) + CGFloat(i) / 2, width: 100, height: 0.5))
            }
        }
        #expect(try picture.rect("/children/0/children/8") == CGRect(x: 0, y: 4, width: 100, height: 4))
        #expect(try picture.pixel(50, 6) == LayoutPicture.swatch(ThemeColor.hex(0x0000ff).color))
    }

    @Test func rowStretchesOnlyAutoCrossSizes() async throws {
        let picture = try await LayoutPicture("""
        root = Col([Row([tall, half, auto, nested], 0, "stretch"),
                    Box([half], "row", 0, 0, null, null, null, 100, 40, null, "stretch")], 0)
        tall = Box([], null, 0, 0, "red", null, null, 10, 80)
        half = Box([], null, 0, 0, "red", null, null, 10, "50%")
        auto = Box([half], null, 0, 0, null, null, null, 10, "auto")
        nested = Box([fixed, half], null, 0, 0, null, null, null, 10, "50%")
        fixed = Box([], null, 0, 0, null, null, null, 10, 20)
        """, width: 100)
        let row = "/children/0/children/"
        #expect(try picture.rect(row + "1") == CGRect(x: 10, y: 0, width: 10, height: 0))
        #expect(try picture.pixel(15, 40).alphaComponent == 0)
        #expect(try picture.rect(row + "2").height == 80)
        #expect(try picture.rect(row + "2/children/0").height == 40)
        #expect(try picture.rect(row + "3").height == 20)
        #expect(try picture.rect(row + "3/children/1").height == 0)
        #expect(try picture.rect("/children/1/children/0").height == 20)
    }

    @Test func wrappingColumnMinimumUsesItsTallestLine() async throws {
        for (wrap, firstHeight) in [(true, 30), (false, 35)] {
            let picture = try await LayoutPicture("""
            root = Col([Box([tiles, blue], null, 0, 0, null, null, null, 100, 40)], 0)
            tiles = Box([a, b], "col", 0, 0, null, null, null, null, 35, null, null, null, \(wrap))
            a = Box([], null, 0, 0, "red", null, null, 10, 30)
            b = Box([], null, 0, 0, "red", null, null, 20, 30)
            blue = Box([], null, 0, 0, "blue", null, null, null, 20)
            """, width: 100)
            let first = try picture.rect("/children/0/children/0")
            let second = try picture.rect("/children/0/children/1")
            #expect(first == CGRect(x: 0, y: 0, width: 100, height: firstHeight))
            #expect(second == CGRect(x: 0, y: firstHeight, width: 100, height: 40 - firstHeight))
            if wrap {
                #expect(try picture.pixel(5, 32) == LayoutPicture.swatch(ThemeColor.hex(0x0000ff).color))
            }
        }
        // Two tiles fit in one column with a four-point gap. The occupied
        // height (10 + 4 + 15) is larger than any individual tile.
        let packed = try await LayoutPicture("""
        root = Col([Box([tiles, sibling], null, 0, 0, null, null, null, 100, 20)], 0)
        tiles = Box([a, b, c], "col", 1, 0, null, null, null, null, 35, null, null, null, true)
        a = Box([], null, 0, 0, null, null, null, 10, 10)
        b = Box([], null, 0, 0, null, null, null, 10, 15)
        c = Box([], null, 0, 0, null, null, null, 10, 16)
        sibling = Box([], null, 0, 0, null, null, null, null, 20)
        """, width: 100)
        #expect(try packed.rect("/children/0/children/0").height == 29)
        #expect(try packed.rect("/children/0/children/1") == CGRect(x: 0, y: 29, width: 100, height: 0))
    }

    @Test func largeFixedWidthsSaturateJustificationAndPlacement() async throws {
        for (width, justify, firstX, blueX, usedWidth) in [
            ("19000000", "end", -33_554_432, 4_445_568, 19_000_000),
            ("1e12", "end", -33_554_432, 33_554_424, 33_554_428),
            ("1e12", "center", -16_777_216, 33_554_432, 33_554_428),
        ] {
            let picture = try await LayoutPicture("""
            root = Col([Row([a, b, blue], 0, null, "\(justify)")], 0)
            a = Box([], null, 0, 0, null, null, null, \(width), 5)
            b = Box([], null, 0, 0, null, null, null, \(width), 9)
            blue = Box([], null, 0, 0, "blue", null, null, 8, 6)
            """, width: 100)
            let first = try picture.rect("/children/0/children/0")
            #expect(first.minX == CGFloat(firstX))
            #expect(first.width == CGFloat(usedWidth))
            let blue = try picture.rect("/children/0/children/2")
            // DOMRect uses float coordinates at the upper LayoutUnit boundary.
            #expect(abs(blue.minX - CGFloat(blueX)) <= 1.0 / 64)
            #expect(try picture.pixel(96, 4).alphaComponent == 0)
        }
        let single = try await LayoutPicture("""
        root = Col([Row([Box([],null,0,0,null,null,null,1e12,6), Box([],null,0,0,"blue",null,null,8,3)], 0, null, "end")])
        """, width: 100)
        #expect(try single.rect("/children/0/children/0").minX == -33_554_336)
        #expect(try single.rect("/children/0/children/1").minX == 92)
    }

    @Test func commentedHexPaintPreservesInheritedInk() async throws {
        for (background, transparent, whiteInk) in [("#0000/*", true, false), ("#000/*x", false, false),
                                                     ("#000", false, true)] {
            let picture = try await LayoutPicture("""
            root = Col([Box([tile], null, 0, 0, "\(background)", null, null, 20, 20)])
            tile = Box([], null, 0, 0, "ink", null, null, 10, 10)
            """)
            let outside = try picture.pixel(15, 15)
            if transparent {
                #expect(outside.alphaComponent == 0)
            } else {
                #expect(try outside == LayoutPicture.swatch(.black))
            }
            let ink = whiteInk ? Color.white : IntelligentTheme.light.ink.color
            #expect(try picture.pixel(5, 5) == LayoutPicture.swatch(ink))
        }
    }
}
