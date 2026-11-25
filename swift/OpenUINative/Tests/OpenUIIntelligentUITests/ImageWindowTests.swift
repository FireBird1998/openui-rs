@testable import OpenUIIntelligentUI
import AppKit
import Darwin
import Testing

extension CollectionDrawingTests {
@Suite @MainActor struct ImageWindowTests {
    @Test(arguments: ["literal", "alias", "state"])
    func streamedImageWaitsForItsFullStatement(_ kind: String) async throws {
        let server = try await PhotoLoopback.start(ipv6: true)
        defer { server.stop() }
        let window = try await CollectionWindow("root = Image(\"\", \"Photo\", 1)", width: 100, height: 120)
        defer { window.window.close() }
        let prefix = switch kind {
        case "alias": "root = Image(src, \"Photo\", 1)\nsrc = (\""
        case "state": "root = Image($src, \"Photo\", 1)\n$src = (\""
        default: "root = Image(\""
        }
        let address = server.url("picture.png")
        let beforeClosure = kind == "literal" ? "\", \"Photo\", 1" : "\""
        for partial in [String(address.dropLast(7)), String(address.dropLast(3)), address, address + beforeClosure] {
            try await window.live.show(response: prefix + partial, streaming: true)
            await window.settle()
        }
        #expect(server.requests.isEmpty, "Premature requests: \(server.requests)")
        let whole = prefix + address + (kind == "literal" ? "\", \"Photo\", 1)" : "\")")
        try await window.live.show(response: whole, streaming: true)
        await window.settle()
        #expect(server.requests.isEmpty)
        try await window.live.show(response: whole + "\n", streaming: true)
        for _ in 0..<30 {
            await window.settle()
            if server.requests.contains("/picture.png") { break }
        }
        #expect(server.requests == ["/picture.png"])
        let photo = try window.rect("/")
        #expect(try await waitForPhoto(window, in: photo))
        let streamedPixel = try PhotoWindowPixels(window).color(in: photo, x: 0.25, y: 0.5)
        try await window.live.show(response: whole, streaming: false)
        let color = try await waitForPhoto(window, in: photo)
        #expect(color)
        let wholePixel = try PhotoWindowPixels(window).color(in: photo, x: 0.25, y: 0.5)
        #expect(PhotoWindowPixels.close(streamedPixel, wholePixel))
        #expect(server.requests == ["/picture.png"])
    }

    @Test func anImageBelowAnUnscrollableWindowMakesNoRequest() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await CollectionWindow("root = Text(\"Viewport\")", width: 150, height: 180)
        defer { window.window.close() }
        // Let the program overflow a fixed window. NSHostingView otherwise
        // passes its intrinsic minimum size through to the containing window.
        window.host.sizingOptions = []
        try await window.live.show(response: """
        root = Col([Box([], null, null, null, null, null, null, null, 3000),
                    Image("\(server.url("below"))", "Below", 1)], 0)
        """, streaming: false)
        await window.settle()
        #expect(window.host.bounds.height == 180)
        #expect(try window.rect("/children/1").minY >= window.host.bounds.height)
        await window.settle()
        #expect(server.requests.isEmpty)
    }

    @Test(arguments: ["SwiftUI", "AppKit", "removal"])
    func leavingTheViewportOrRemovingTheImageClosesItsRequest(_ host: String) async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await CollectionWindow("root = Image(\"\", \"Photo\", 1)",
                                                width: 150, height: 180, scrollable: host == "SwiftUI")
        defer { window.window.close() }
        let nativeScroll = host == "AppKit" ? embedInAppKitScrollView(window, documentHeight: 800) : nil
        let top = "Image(\"\(server.url("hold-top"))\", \"Top\", 1)"
        let source = host == "removal" ? "root = \(top)" : """
        root = Col([\(top),
                    Box([], null, null, null, null, null, null, null, 500),
                    Image("\(server.url("lower"))", "Lower", 1)], 0)
        """
        try await window.live.show(response: source, streaming: false)
        for _ in 0..<20 {
            await window.settle()
            if server.metric("hold-top") == "1" { break }
        }
        #expect(server.requests == ["/hold-top"])
        #expect(server.metric("hold-top") == "1")
        if host == "removal" {
            try await window.live.show(response: "root = Text(\"Removed\")", streaming: false)
        } else {
            let scroll = try #require(nativeScroll ?? window.scrollView)
            scrollTo(scroll, y: 300)
        }
        for _ in 0..<12 {
            await window.settle()
            if server.metric("hold-top") == "0" { break }
        }
        #expect(server.metric("hold-top") == "0")
        if host != "removal" {
            let scroll = try #require(nativeScroll ?? window.scrollView)
            scrollTo(scroll, y: 650)
            for _ in 0..<20 {
                await window.settle()
                if server.requests.contains("/lower") { break }
            }
            #expect(server.requests.contains("/lower"))
        }
    }

    @Test func aPageLimitsConcurrentDownloads() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let photos = (0..<12).map { "Image(\"\(server.url("busy-\($0)"))\", \"Photo\", 4)" }
        let window = try await CollectionWindow("root = Col([\(photos.joined(separator: ","))], 0)",
                                                width: 80, height: 260)
        defer { window.window.close() }
        for _ in 0..<45 {
            await window.settle()
            if server.requests.count == photos.count { break }
        }
        #expect(server.requests.count == photos.count)
        let peak = try #require(server.metric("peak").flatMap(Int.init))
        print("photo-server peak=\(peak) active loads")
        #expect((1...4).contains(peak), "Peak active HTTP loads: \(peak)")
    }

    @Test func leavingDistantRowsReleasesDownsampledRastersAndRevisitingReloads() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let photos = (0..<24).map { "Image(\"\(server.url("square-\($0)"))\", \"Photo\", 1, 0)" }
        let before = imageMemory()
        let window = try await CollectionWindow("root = Col([\(photos.joined(separator: ","))], 0)",
                                                width: 150, height: 165, scrollable: true)
        defer { window.window.close() }
        let scroll = try #require(window.scrollView)
        let rows = try photos.indices.map { try window.rect("/children/\($0)") }
        for (index, rect) in rows.enumerated() {
            scrollTo(scroll, y: rect.minY)
            for _ in 0..<20 {
                await window.settle()
                if server.requests.contains("/square-\(index)") { break }
            }
            await window.settle()
            #expect(server.requests.contains("/square-\(index)"))
        }
        let after = imageMemory()
        print("photo-memory 24x1024 before=\(before) after=\(after)")
        #expect(after.resident - before.resident < 80 * 1024 * 1024)
        let firstVisits = server.requests.filter { $0 == "/square-0" }.count
        scrollTo(scroll, y: 0)
        for _ in 0..<20 {
            await window.settle()
            if server.requests.filter({ $0 == "/square-0" }).count > firstVisits { break }
        }
        #expect(server.requests.filter { $0 == "/square-0" }.count == firstVisits + 1)
    }

    @Test func aMountedImageReturnsItsRasterAllowanceWhenScrolledAway() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await CollectionWindow("root = Text(\"Viewport\")", width: 150, height: 180)
        defer { window.window.close() }
        let scroll = embedInAppKitScrollView(window, documentHeight: 800)
        try await window.live.show(response: """
        root = Col([Image("\(server.url("square"))", "Photo", 1),
                    Box([], null, null, null, null, null, null, null, 500)], 0)
        """, streaming: false)
        await window.settle()
        let photo = try window.rect("/children/0")
        try #require(try await waitForPhoto(window, in: photo))
        let budget = PhotoBudget.inWindow(window.window)
        try #require((1..<(8 * 1024 * 1024)).contains(budget.charged))
        scrollTo(scroll, y: 300)
        for _ in 0..<12 {
            await window.settle()
            if budget.charged == 0 { break }
        }
        #expect(window.host.window === window.window)
        #expect(try window.rect("/children/0") == photo)
        #expect(budget.charged == 0)
        #expect(server.requests == ["/square"])
    }

    @Test func aLargeSixteenBitImageDrawsWithinTheDecodeAllowance() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let before = imageMemory()
        let window = try await CollectionWindow("root = Image(\"\(server.url("large16"))\", \"Photo\", 1)",
                                                width: 150, height: 165)
        defer { window.window.close() }
        #expect(try await waitForPhoto(window, in: window.rect("/")))
        let after = imageMemory()
        print("photo-memory 4096x4096 16-bit before=\(before) after=\(after)")
        #expect(after.peak - max(before.peak, before.resident) < 128 * 1024 * 1024)
    }

    @Test(arguments: ["animated", "many-frames"])
    func animatedPhotoPlaysAndAFrameCountOverTheCapUsesTheFirstFrame(_ path: String) async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await CollectionWindow("root = Image(\"\(server.url(path))\", \"GIF\", 1)",
                                                width: 100, height: 120)
        defer { window.window.close() }
        let photo = try window.rect("/")
        var red = false, blue = false
        for _ in 0..<18 {
            await window.settle()
            let pixel = try PhotoWindowPixels(window).color(in: photo, x: 0.5, y: 0.5)
            red = red || (pixel.redComponent > 0.85 && pixel.blueComponent < 0.15)
            blue = blue || (pixel.blueComponent > 0.85 && pixel.redComponent < 0.15)
        }
        #expect(red)
        #expect(blue == (path == "animated"))
    }

    @Test func aCoveredSVGDrawsAtItsNormalizedViewport() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        // With no declared size, the Image viewport is 300 × 150. CoreSVG
        // receives explicit dimensions and a viewBox from the rebuilder.
        // Drawing the original bytes gives CoreSVG a different default
        // viewport, so the red rectangle extends into the blue sample.
        try server.serveSVG(svgDocument("""
        <rect width="100" height="150" fill="blue"/>
        <rect width="75" height="150" style="fill:red"/>
        """, size: ""), as: "covered")
        let window = try await CollectionWindow("""
        root = Image("\(server.url("vector-covered"))", "Diagram", 2, 0, "contain")
        """, width: 100, height: 70)
        defer { window.window.close() }
        let photo = try window.rect("/")
        var matched = false
        for _ in 0..<25 {
            await window.settle()
            let pixels = try PhotoWindowPixels(window)
            let left = try pixels.color(in: photo, x: 0.15, y: 0.5)
            let right = try pixels.color(in: photo, x: 0.30, y: 0.5)
            if left.redComponent > 0.85, left.blueComponent < 0.15,
               right.blueComponent > 0.85, right.redComponent < 0.15 {
                matched = true
                break
            }
        }
        #expect(matched)
        #expect(server.requests == ["/vector-covered"])
    }

    @Test(arguments: ["style", "class", "use", "symbol", "gradient-href", "mask", "pattern",
                      "marker", "filter", "nested-svg", "data-image", "font-family", "inline-css",
                      "missing-reference", "active"])
    func anUnsupportedSVGDrawsTheEntireThemedEmptyState(_ feature: String) async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let fragment = switch feature {
        case "style": #"<style>rect { fill: blue }</style>"#
        case "class": #"<rect class="color" width="64" height="32"/>"#
        case "use": ##"<use href="#shape"/>"##
        case "symbol": #"<defs><symbol id="shape"><rect width="64" height="32"/></symbol></defs>"#
        case "gradient-href":
            ##"<defs><linearGradient id="base"/><linearGradient id="paint" xlink:href="#base"/></defs>"##
        case "mask": #"<mask id="shape"><rect width="32" height="32" fill="white"/></mask>"#
        case "pattern": #"<pattern id="paint" width="32" height="32"><rect width="16" height="16"/></pattern>"#
        case "marker": #"<marker id="arrow"><path d="M0 0L4 2L0 4Z"/></marker>"#
        case "filter": #"<filter id="blur"><feGaussianBlur stdDeviation="2"/></filter>"#
        case "nested-svg": #"<svg width="32" height="32"><rect width="32" height="32" fill="blue"/></svg>"#
        case "data-image":
            #"<image width="64" height="32" href="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"/>"#
        case "font-family": #"<text x="5" y="20" font-family="monospace">OpenUI</text>"#
        case "inline-css": #"<rect width="64" height="32" style="fill:blue !important;fill:red"/>"#
        case "missing-reference": #"<rect width="64" height="32" fill="url(#absent)"/>"#
        default: ""
        }
        let path = feature == "active" ? "svg-active" : "vector-" + feature
        if feature != "active" {
            try server.serveSVG(svgDocument(redSVGRectangle + fragment), as: feature)
        }
        let window = try await CollectionWindow("""
        root = Col([Image("\(server.url(path))", " Unsupported diagram ", 2, 0),
                    Image("", "Empty reference", 2, 0)], 0)
        """, width: 100, height: 110)
        defer { window.window.close() }
        let photo = try window.rect("/children/0")
        let empty = try window.rect("/children/1")
        var matched = false
        for _ in 0..<20 {
            await window.settle()
            let pixels = try PhotoWindowPixels(window)
            if try pixels.drawsEmptyGlyph(in: photo), try pixels.drawsEmptyGlyph(in: empty) {
                let actual = try pixels.color(in: photo, x: 0.2, y: 0.2)
                let reference = try pixels.color(in: empty, x: 0.2, y: 0.2)
                matched = PhotoWindowPixels.close(actual, reference)
                if matched { break }
            }
        }
        #expect(matched, "Unsupported SVG drew a partial picture: \(feature)")
        #expect(imageLabels(in: window).sorted() == ["Empty reference", "Unsupported diagram"])
        #expect(server.requests == ["/" + path])
    }

    @Test func decorativeLoadedImagesAreHiddenButTheEmptyStateRetainsItsImageRole() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await CollectionWindow("""
        root = Col([Image("\(server.url("decorative"))", "", 2),
                    Image("\(server.url("spaces"))", "   ", 2),
                    Image("\(server.url("named"))", " Named ", 2),
                    Image("\(server.url("broken"))", "", 2)], 0)
        """, width: 100, height: 210)
        defer { window.window.close() }
        for index in 0..<3 {
            #expect(try await waitForPhoto(window, in: window.rect("/children/\(index)")))
        }
        #expect(imageLabels(in: window).sorted() == ["", "Named"])
    }

    private func waitForPhoto(_ window: CollectionWindow, in photo: CGRect) async throws -> Bool {
        for _ in 0..<25 {
            await window.settle()
            let color = try PhotoWindowPixels(window).color(in: photo, x: 0.25, y: 0.5)
            if color.redComponent > 0.85, color.blueComponent < 0.15 { return true }
        }
        return false
    }

    private func embedInAppKitScrollView(_ window: CollectionWindow, documentHeight: CGFloat) -> NSScrollView {
        let scroll = NSScrollView(frame: window.host.frame)
        window.window.contentView = scroll
        window.host.autoresizingMask = [.width]
        window.host.setFrameSize(CGSize(width: scroll.contentSize.width, height: documentHeight))
        scroll.documentView = window.host
        scrollTo(scroll, y: 0)
        return scroll
    }

    private func scrollTo(_ view: NSScrollView, y: CGFloat) {
        let clip = view.contentView
        clip.scroll(to: CGPoint(x: 0, y: y))
        view.reflectScrolledClipView(clip)
    }

    @Test func imageDefersAnOffscreenRequestAndNeverRequestsAProtocolRelativeAddress() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let relative = String(server.url("refused").dropFirst("http:".count))
        let window = try await CollectionWindow("""
        root = Col([Image("\(relative)", "Refused", 2),
                    Box([], null, null, null, null, null, null, null, 250),
                    Image("\(server.url("later"))", "Later", 2)], 0)
        """, width: 180, height: 110, scrollable: true)
        defer { window.window.close() }
        let photo = try window.rect("/children/2")
        #expect(photo.minY >= window.host.bounds.height)
        await window.settle()
        #expect(server.requests.isEmpty)
        let scroll = try #require(window.scrollView)
        let clip = scroll.contentView
        clip.scroll(to: CGPoint(x: 0, y: photo.minY))
        scroll.reflectScrolledClipView(clip)
        for _ in 0..<30 {
            await window.settle()
            if server.requests.contains("/later") { break }
        }
        #expect(server.requests == ["/later"])
    }

    @Test func loopbackPhotoDrawsCoverAndContainAndFailureDrawsTheEmptyState() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await CollectionWindow("""
        root = Col([Image("\(server.url("cover"))", " Cover ", 1, 12),
                    Image("\(server.url("contain"))", "Contain", 1, 0, "contain"),
                    Image("\(server.url("broken"))", " Missing ", 1, 0),
                    Image("", "Empty", 1, 0)], 0)
        """, width: 120, height: 490)
        defer { window.window.close() }
        let cover = try window.rect("/children/0")
        let contain = try window.rect("/children/1")
        let failed = try window.rect("/children/2")
        let empty = try window.rect("/children/3")
        var matched = false
        for _ in 0..<40 {
            await window.settle()
            let pixels = try PhotoWindowPixels(window)
            let coverRed = try pixels.color(in: cover, x: 0.25, y: 0.1)
            let containRed = try pixels.color(in: contain, x: 0.25, y: 0.5)
            let outsideContain = try pixels.color(in: contain, x: 0.25, y: 0.1)
            let background = try pixels.color(in: empty, x: 0.25, y: 0.1)
            let failureMatches = try pixels.drawsEmptyGlyph(in: failed) && pixels.drawsEmptyGlyph(in: empty)
            if coverRed.redComponent > 0.9, coverRed.blueComponent < 0.1,
               containRed.redComponent > 0.9, containRed.blueComponent < 0.1,
               PhotoWindowPixels.close(outsideContain, background), failureMatches {
                let coverBlue = try pixels.color(in: cover, x: 0.75, y: 0.1)
                let containBlue = try pixels.color(in: contain, x: 0.75, y: 0.5)
                let outsideRadius = try pixels.color(in: cover, x: 0.015, y: 0.015)
                #expect(coverBlue.blueComponent > 0.9 && coverBlue.redComponent < 0.1)
                #expect(containBlue.blueComponent > 0.9 && containBlue.redComponent < 0.1)
                #expect(abs(outsideRadius.redComponent - outsideRadius.blueComponent) < 0.1)
                matched = true
                break
            }
        }
        #expect(matched, "Cover, contain and the failed image did not settle to their expected pixels")
        #expect(server.requests.sorted() == ["/broken", "/contain", "/cover"])
        #expect(imageLabels(in: window).sorted() == ["Contain", "Cover", "Empty", "Missing"])
    }

    @Test func replacingAnImageSourceLoadsTheNewPhoto() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await CollectionWindow("""
        root = Col([Image("\(server.url("stalled"))", "Old", 1),
                    Image("", "Empty reference", 1)], 0)
        """, width: 100, height: 220)
        defer { window.window.close() }
        for _ in 0..<25 {
            if server.requests.contains("/stalled") { break }
            await window.settle()
        }
        try #require(server.requests == ["/stalled"])
        let rect = try window.rect("/children/0")
        #expect(try !PhotoWindowPixels(window).drawsEmptyGlyph(in: rect))

        try await window.live.show(response: """
        root = Col([Image("\(server.url("new"))", "New", 1),
                    Image("", "Empty reference", 1)], 0)
        """, streaming: false)
        var drewNew = false
        for _ in 0..<35 {
            await window.settle()
            let color = try PhotoWindowPixels(window).color(in: rect, x: 0.75, y: 0.5)
            if color.blueComponent > 0.9, color.redComponent < 0.1 {
                drewNew = true
                break
            }
        }
        #expect(drewNew)
        #expect(server.requests == ["/stalled", "/new"])
        #expect(imageLabels(in: window).sorted() == ["Empty reference", "New"])
    }

    @Test func returningToTheMostRecentlyFailedSourceShowsTheEmptyStateWithoutRetrying() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let failed = server.url("broken")
        let window = try await CollectionWindow("""
        $source = "\(failed)"
        root = Col([Image($source, "Photo", 1), Image("", "Empty reference", 1)], 0)
        """, width: 100, height: 220)
        defer { window.window.close() }
        let photo = try window.rect("/children/0")
        var drewEmpty = false
        for _ in 0..<25 {
            await window.settle()
            if try PhotoWindowPixels(window).drawsEmptyGlyph(in: photo) {
                drewEmpty = true
                break
            }
        }
        try #require(server.requests == ["/broken"])
        try #require(drewEmpty)

        try window.live.set("$source", to: .string(server.url("new")))
        var drewNew = false
        for _ in 0..<25 {
            await window.settle()
            let color = try PhotoWindowPixels(window).color(in: photo, x: 0.75, y: 0.5)
            if color.blueComponent > 0.9, color.redComponent < 0.1 {
                drewNew = true
                break
            }
        }
        try #require(drewNew)

        try window.live.set("$source", to: .string(failed))
        for _ in 0..<3 { await window.settle() }
        #expect(try PhotoWindowPixels(window).drawsEmptyGlyph(in: photo))
        #expect(server.requests == ["/broken", "/new"])
    }

    private func imageLabels(in window: CollectionWindow) -> [String] {
        var pending: [AnyObject] = [window.host]
        var labels: [String] = []
        while let node = pending.popLast() {
            if node.accessibilityRole?() == .image, let label = node.accessibilityLabel?() {
                labels.append(label)
            }
            pending.append(contentsOf: (node.accessibilityChildren?() ?? []).map { $0 as AnyObject })
        }
        return labels
    }
}
}

func imageMemory() -> (resident: Int64, peak: Int64) {
    var usage = rusage()
    getrusage(RUSAGE_SELF, &usage)
    var info = mach_task_basic_info()
    let capacity = MemoryLayout<mach_task_basic_info>.size / MemoryLayout<integer_t>.size
    var count = mach_msg_type_number_t(capacity)
    let status = withUnsafeMutablePointer(to: &info) {
        $0.withMemoryRebound(to: integer_t.self, capacity: capacity) {
            task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
        }
    }
    let peak = Int64(usage.ru_maxrss)
    return (status == KERN_SUCCESS ? Int64(info.resident_size) : peak, peak)
}

@MainActor private struct PhotoWindowPixels {
    let bitmap: NSBitmapImageRep
    let bounds: CGRect

    init(_ view: CollectionWindow) throws {
        bounds = view.host.bounds
        bitmap = try #require(view.host.bitmapImageRepForCachingDisplay(in: bounds))
        view.host.cacheDisplay(in: bounds, to: bitmap)
    }

    func color(in rect: CGRect, x: CGFloat, y: CGFloat) throws -> NSColor {
        let column = Int((rect.minX + x * rect.width) * CGFloat(bitmap.pixelsWide) / bounds.width)
        let row = Int((rect.minY + y * rect.height) * CGFloat(bitmap.pixelsHigh) / bounds.height)
        return try #require(bitmap.colorAt(x: column, y: row)?.usingColorSpace(.sRGB))
    }

    static func close(_ lhs: NSColor, _ rhs: NSColor) -> Bool {
        abs(lhs.redComponent - rhs.redComponent) < 0.04 &&
        abs(lhs.greenComponent - rhs.greenComponent) < 0.04 &&
        abs(lhs.blueComponent - rhs.blueComponent) < 0.04 &&
        abs(lhs.alphaComponent - rhs.alphaComponent) < 0.04
    }

    func drawsEmptyGlyph(in rect: CGRect) throws -> Bool {
        // Find the glyph's left stroke between two background samples. The
        // measured frame locates the stroke to within one point on either scale.
        func at(_ offset: CGFloat) throws -> NSColor {
            try color(in: rect, x: 0.5 + offset / rect.width, y: 0.5)
        }
        let outside = try at(-13), inside = try at(-7)
        guard Self.close(outside, inside) else { return false }
        return try [-11, -10, -9].contains {
            let stroke = try at($0)
            return !Self.close(stroke, outside) && !Self.close(stroke, inside)
        }
    }
}
