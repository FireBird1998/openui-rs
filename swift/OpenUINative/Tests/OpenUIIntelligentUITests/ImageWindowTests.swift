@testable import OpenUIIntelligentUI
import AppKit
import Darwin
import SwiftTestSupport
import Testing

extension CollectionDrawingTests {
@Suite(NativeTestWatchdog()) @MainActor struct ImageWindowTests {
    init() { PhotoBudgetAudit.install() }

    @Test(arguments: ["literal", "alias", "state"])
    func streamedImageWaitsForItsFullStatement(_ kind: String) async throws {
        let server = try await PhotoLoopback.start(ipv6: true)
        defer { server.stop() }
        let window = try await mountPhotos("root = Image(\"\", \"Photo\", 1)", width: 100, height: 120)
        defer { window.window.closeForTest() }
        let prefix = switch kind {
        case "alias": "root = Image(src, \"Photo\", 1)\nsrc = (\""
        case "state": "root = Image($src, \"Photo\", 1)\n$src = (\""
        default: "root = Image(\""
        }
        let address = server.url("picture.png")
        let beforeClosure = kind == "literal" ? "\", \"Photo\", 1" : "\""
        for partial in [String(address.dropLast(7)), String(address.dropLast(3)), address, address + beforeClosure] {
            try await window.live.show(response: prefix + partial, streaming: true)
            try await window.waitForProgramDrawing()
            #expect(window.live.hasUnfinishedStatements)
            try await requirePhotoQuiet("no request while the stream statement is incomplete", in: window,
                                        server: server, expected: [], for: .milliseconds(100))
        }
        let whole = prefix + address + (kind == "literal" ? "\", \"Photo\", 1)" : "\")")
        try await window.live.show(response: whole, streaming: true)
        try await window.waitForProgramDrawing()
        #expect(window.live.hasUnfinishedStatements)
        try await requirePhotoQuiet("no request before the stream statement ends", in: window,
                                    server: server, expected: [], for: .milliseconds(100))
        try await window.live.show(response: whole + "\n", streaming: true)
        try await waitForPhotoEvent("the settled stream's image request", in: window) {
            server.requests.contains("/picture.png")
        }
        #expect(server.requests == ["/picture.png"])
        let photo = try await photoRect("/", in: window)
        // Compare the two drawings once each has finished its 0.3 s fade-in.
        try await waitForOpaquePhoto(window, in: photo)
        let streamedPixel = try PhotoWindowPixels(window).color(in: photo, x: 0.25, y: 0.5)
        try await window.live.show(response: whole, streaming: false)
        try await window.waitForProgramDrawing()
        try await waitForOpaquePhoto(window, in: photo)
        let wholePixel = try PhotoWindowPixels(window).color(in: photo, x: 0.25, y: 0.5)
        #expect(PhotoWindowPixels.close(streamedPixel, wholePixel))
        try await requirePhotoQuiet("no duplicate request after finishing the stream", in: window,
                                    server: server, expected: ["/picture.png"], for: .milliseconds(100))
    }

    @Test func anImageBelowAnUnscrollableWindowMakesNoRequest() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Text(\"Viewport\")", width: 150, height: 180)
        defer { window.window.closeForTest() }
        // Let the program overflow a fixed window. NSHostingView otherwise
        // passes its intrinsic minimum size through to the containing window.
        window.host.sizingOptions = []
        try await window.live.show(response: """
        root = Col([Box([], null, null, null, null, null, null, null, 3000),
                    Image("\(server.url("below"))", "Below", 1)], 0)
        """, streaming: false)
        try await window.waitForProgramDrawing()
        #expect(window.host.bounds.height == 180)
        #expect(try await photoRect("/children/1", in: window).minY >= window.host.bounds.height)
        try await requirePhotoQuiet("no request for the clipped image", in: window,
                                    server: server, expected: [], for: .milliseconds(200))
    }

    @Test(arguments: ["SwiftUI", "AppKit", "removal"])
    func leavingTheViewportOrRemovingTheImageClosesItsRequest(_ host: String) async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Image(\"\", \"Photo\", 1)",
                                                width: 150, height: 180, scrollable: host == "SwiftUI")
        defer { window.window.closeForTest() }
        let nativeScroll = host == "AppKit" ? embedInAppKitScrollView(window, documentHeight: 800) : nil
        let top = "Image(\"\(server.url("hold-top"))\", \"Top\", 1)"
        let source = host == "removal" ? "root = \(top)" : """
        root = Col([\(top),
                    Box([], null, null, null, null, null, null, null, 500),
                    Image("\(server.url("lower"))", "Lower", 1)], 0)
        """
        try await window.live.show(response: source, streaming: false)
        try await waitForPhotoEvent("the top image's held response", in: window) {
            server.metric("hold-top") == "1"
        }
        #expect(server.requests == ["/hold-top"])
        if host == "removal" {
            try await window.live.show(response: "root = Text(\"Removed\")", streaming: false)
        } else {
            let scroll = try #require(nativeScroll ?? window.scrollView)
            scrollTo(scroll, y: 300)
        }
        try await waitForPhotoEvent("the top image's connection to close", in: window) {
            server.metric("hold-top").map { $0 != "1" } == true
        }
        #expect(server.metric("hold-top") == "0", "The held connection must close on the client side")
        if host != "removal" {
            let scroll = try #require(nativeScroll ?? window.scrollView)
            scrollTo(scroll, y: 650)
            try await waitForPhotoEvent("the lower image's request", in: window) {
                server.requests.contains("/lower")
            }
        }
    }

    @Test func aPageLimitsConcurrentDownloads() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let paths = (0..<12).map { "busy-\($0)" }
        let photos = paths.map { "Image(\"\(server.url($0))\", \"Photo\", 4)" }
        let window = try await mountPhotos("root = Col([\(photos.joined(separator: ","))], 0)",
                                                width: 80, height: 260)
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        // The page allows four downloads at once. The literal is deliberate: a
        // change to the production limit must fail here rather than move the
        // expectation with it.
        try await waitForPhotoEvent("every photo admitted or queued, and each admitted download held", in: window) {
            budget.active > 0 && budget.active + budget.queued == paths.count &&
                server.metric("peak").flatMap(Int.init) == budget.active
        }
        let heldPeak = try #require(server.metric("peak").flatMap(Int.init))
        #expect(heldPeak == 4, "Peak active HTTP loads before release: \(heldPeak)")
        #expect(budget.queued == 8)
        #expect(server.requests.count == 4)
        try server.releaseBusyResponses()
        try await waitForPhotoEvent("all \(photos.count) photo requests", in: window) {
            server.requests.count >= photos.count
        }
        #expect(server.requests.sorted() == paths.map { "/" + $0 }.sorted())
        let peak = try #require(server.metric("peak").flatMap(Int.init))
        print("photo-server peak=\(peak) active loads")
        #expect(peak == 4, "Peak active HTTP loads: \(peak)")
        try await waitForPhotoEvent("every download slot to return after the queue drains", in: window) {
            budget.active == 0 && budget.queued == 0
        }
    }

    @Test(arguments: ["close", "remove"])
    func closingOrRemovingPhotosCancelsDownloadsAndDropsWaiters(_ action: String) async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        // The literal is deliberate: a change to the production limit must
        // fail here rather than move the expectation with it.
        let slots = 4
        let oldPaths = (0..<(slots * 2)).map { "hold-old-\($0)" }
        let newPaths = (0..<slots).map { "hold-fresh-\($0)" }
        func program(_ paths: [String]) -> String {
            let photos = paths.map { "Image(\"\(server.url($0))\", \"Photo\", 4)" }
            return "root = Col([\(photos.joined(separator: ","))], 0)"
        }
        let window = try await mountPhotos(program(oldPaths), width: 80, height: 260)
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        try await waitForPhotoEvent("four held downloads and four queued photos", in: window) {
            budget.active == slots && budget.queued == slots &&
            oldPaths.filter { server.metric($0) == "1" }.count == slots
        }
        let started = server.requests
        try #require(started.count == slots)
        if action == "close" {
            window.window.closeForTest()
            #expect(!window.window.isVisible)
            // Closing returns every reservation's slot and bytes in the close
            // callback itself, whatever stage it reached, before any of the
            // cancelled loads unwinds.
            #expect(budget.active == 0)
            #expect(budget.charged == 0)
            #expect(budget.queued == 0)
        } else {
            try await window.live.show(response: "root = Text(\"Removed\")", streaming: false)
        }
        try await waitForPhotoEvent("all four old connections to close", in: window) {
            started.allSatisfy { path in
                server.metric(String(path.dropFirst())).map { $0 != "1" } == true
            }
        }
        for path in started {
            #expect(server.metric(String(path.dropFirst())) == "0", "The client must close \(path)")
        }
        try await waitForPhotoEvent("the cancelled window's raster charges and queue to drain", in: window) {
            budget.charged == 0 && budget.queued == 0
        }
        #expect(budget.active == 0)
        #expect(server.requests == started)

        try await window.live.show(response: program(newPaths), streaming: false)
        if action == "close" { window.window.orderFrontRegardless() }
        try await waitForPhotoEvent("four fresh stalled downloads", in: window) {
            newPaths.allSatisfy { server.metric($0) == "1" }
        }
        // Closing ends a budget for good. The reopened window charges a fresh
        // budget and the closed one stays empty; removal keeps the budget.
        let current = PhotoBudget.inWindow(window.window)
        if action == "close" {
            #expect(current !== budget)
            #expect(budget.closed)
            #expect(budget.active == 0 && budget.charged == 0 && budget.queued == 0)
        } else {
            #expect(current === budget)
        }
        #expect(current.active == slots)
        #expect(current.queued == 0)
        #expect(server.requests.sorted() == (started + newPaths.map { "/" + $0 }).sorted())
    }

    @Test func cancellingAQueuedWaiterKeepsAllDownloadSlotsAvailable() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        // The literal is deliberate, as in aPageLimitsConcurrentDownloads.
        let slots = 4
        let paths = (0..<slots).map { "busy-active-\($0)" }
        let photos = paths.map { "Image(\"\(server.url($0))\", \"Held\", 4)" }
        let window = try await mountPhotos("root = Col([\(photos.joined(separator: ","))], 0)", width: 80, height: 260)
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        try await waitForPhotoEvent("all slots to be held before queueing a waiter", in: window) {
            server.metric("peak") == String(slots) && budget.active == slots
        }

        let scope = budget.makeScope()
        let waiter = Task { await scope.reserve() }
        defer {
            waiter.cancel()
            scope.cancel()
        }
        try await waitForPhotoEvent("the cancellable waiter to enter the full queue", in: window) { budget.queued == 1 }
        waiter.cancel()
        try await waitForPhotoEvent("the cancelled waiter to leave while all slots are still held", in: window) {
            budget.queued == 0
        }
        #expect(await waiter.value == nil)
        #expect(budget.active == slots)

        try server.releaseBusyResponses()
        try await waitForPhotoEvent("the held downloads to return all their slots", in: window) { budget.active == 0 }
        #expect(server.requests.sorted() == paths.map { "/" + $0 }.sorted())
    }

    @Test func aSuccessfulRasterReturnsExactlyOneDownloadSlot() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Image(\"\(server.url("success"))\", \"Photo\", 1)",
                                                width: 100, height: 120)
        defer { window.window.closeForTest() }
        try await waitForPhoto(window, in: photoRect("/", in: window))
        let budget = PhotoBudget.inWindow(window.window)
        #expect((1..<PhotoLimits.decodedImageBytes).contains(budget.charged))
        #expect(budget.active == 0)
        #expect(server.requests == ["/success"])
    }

    @Test func retainingRasterCannotAuthorizeAFifthDownload() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Text(\"Budget\")", width: 100, height: 120)
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        let scope = budget.makeScope()
        defer { scope.cancel() }
        let size = try #require(PhotoSize(points: CGSize(width: 64, height: 64), scale: 1))
        func source(_ path: String) throws -> PhotoSource { try #require(PhotoSource(server.url(path))) }

        // Two reservations spend their slots on successful loads.
        var spent: [PhotoBudget.Reservation] = []
        for path in ["spent-0", "spent-1"] {
            let reservation = try #require(await scope.reserve())
            #expect(await reservation.load(try source(path), size: size))
            spent.append(reservation)
        }
        #expect(budget.active == 0)

        // Three fresh reservations hold three downloads at the fixture.
        var heldSources: [PhotoSource] = []
        for index in 0..<3 { heldSources.append(try source("busy-held-\(index)")) }
        var held: [Task<Bool, Never>] = []
        for heldSource in heldSources {
            let reservation = try #require(await scope.reserve())
            held.append(Task { await reservation.load(heldSource, size: size) })
        }
        try await waitForPhotoEvent("three held downloads", in: window) {
            server.metric("peak") == "3" && budget.active == 3
        }

        // Reusing a spent reservation must neither request nor take a slot.
        let reuseSources = try (0..<2).map { try source("busy-reuse-\($0)") }
        let refused = ReuseOutcomes()
        for (reservation, reuseSource) in zip(spent, reuseSources) {
            Task { refused.values.append(await reservation.load(reuseSource, size: size) == false) }
        }
        try await waitForPhotoEvent("both spent reservations to refuse another load", in: window) {
            try #require(!server.requests.contains { $0.hasPrefix("/busy-reuse") },
                         "A spent reservation started a download: \(server.requests)")
            return refused.values.count == 2
        }
        #expect(refused.values == [true, true])
        #expect(budget.active == 3)
        #expect(server.metric("peak") == "3")

        try server.releaseBusyResponses()
        for load in held { _ = await load.value }
        #expect(budget.active == 0)
        #expect(server.requests.sorted() == ["/busy-held-0", "/busy-held-1", "/busy-held-2", "/spent-0", "/spent-1"])
    }

    @Test func idleReservationCannotKeepSlotAfterWindowLifetimeEnds() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Text(\"Budget\")", width: 100, height: 120)
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        let scope = budget.makeScope()
        let reservation = try #require(await scope.reserve())
        #expect(budget.active == 1)
        #expect(budget.charged == PhotoLimits.decodedImageBytes)

        // The reservation is registered with its scope from acquisition, so
        // closing the window returns its slot and bytes although it never
        // started a load and its handle is still alive.
        window.window.closeForTest()
        #expect(budget.active == 0)
        #expect(budget.charged == 0)
        let size = try #require(PhotoSize(points: CGSize(width: 64, height: 64), scale: 1))
        let after = try #require(PhotoSource(server.url("after-close")))
        #expect(await reservation.load(after, size: size) == false)
        #expect(budget.active == 0)
        #expect(budget.charged == 0)
        #expect(server.requests.isEmpty)
        withExtendedLifetime(reservation) {}
    }

    @Test func leavingDistantRowsReleasesDownsampledRastersAndRevisitingReloads() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let photos = (0..<24).map { "Image(\"\(server.url("square-\($0)"))\", \"Photo\", 1, 0)" }
        let before = imageMemory()
        let window = try await mountPhotos("root = Col([\(photos.joined(separator: ","))], 0)",
                                                width: 150, height: 165, scrollable: true)
        defer { window.window.closeForTest() }
        let scroll = try #require(window.scrollView)
        try await waitForPhotoEvent("the layout of all 24 rows", in: window) {
            photos.indices.allSatisfy { window.frame(at: "/children/\($0)") != nil }
        }
        let rows = try photos.indices.map { try window.rect("/children/\($0)") }
        for (index, rect) in rows.enumerated() {
            scrollTo(scroll, y: rect.minY)
            let path = "/square-\(index)"
            try await waitForPhotoEvent("the request and pixels for \(path)", in: window) {
                guard server.requests.contains(path) else { return false }
                let visibleRow = try window.rect("/children/\(index)")
                let sample = CGPoint(x: visibleRow.minX + visibleRow.width * 0.25, y: visibleRow.midY)
                guard window.host.bounds.contains(sample) else { return false }
                return try PhotoWindowPixels(window).color(in: visibleRow, x: 0.25, y: 0.5).isPhotoRed
            }
        }
        let after = imageMemory()
        print("photo-memory 24x1024 before=\(before) after=\(after)")
        #expect(after.resident - before.resident < 80 * 1024 * 1024)
        let firstVisits = server.requests.filter { $0 == "/square-0" }.count
        scrollTo(scroll, y: 0)
        try await waitForPhotoEvent("a fresh request on revisiting the first row", in: window) {
            server.requests.filter { $0 == "/square-0" }.count > firstVisits
        }
        #expect(server.requests.filter { $0 == "/square-0" }.count == firstVisits + 1)
    }

    @Test func aMountedImageReturnsItsRasterAllowanceWhenScrolledAway() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Text(\"Viewport\")", width: 150, height: 180)
        defer { window.window.closeForTest() }
        let scroll = embedInAppKitScrollView(window, documentHeight: 800)
        try await window.live.show(response: """
        root = Col([Image("\(server.url("square"))", "Photo", 1),
                    Box([], null, null, null, null, null, null, null, 500)], 0)
        """, streaming: false)
        try await window.waitForProgramDrawing()
        let photo = try await photoRect("/children/0", in: window)
        try await waitForPhoto(window, in: photo)
        let budget = PhotoBudget.inWindow(window.window)
        try #require((1..<(8 * 1024 * 1024)).contains(budget.charged))
        scrollTo(scroll, y: 300)
        try await waitForPhotoEvent("the scrolled-out raster allowance to be returned", in: window) {
            budget.charged == 0
        }
        #expect(window.host.window === window.window)
        #expect(try window.rect("/children/0") == photo)
        #expect(server.requests == ["/square"])
    }

    @Test func aLargeSixteenBitImageDrawsWithinTheDecodeAllowance() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let before = imageMemory()
        let window = try await mountPhotos("root = Image(\"\(server.url("large16"))\", \"Photo\", 1)",
                                                width: 150, height: 165)
        defer { window.window.closeForTest() }
        try await waitForPhoto(window, in: photoRect("/", in: window))
        let after = imageMemory()
        print("photo-memory 4096x4096 16-bit before=\(before) after=\(after)")
        #expect(after.peak - max(before.peak, before.resident) < 128 * 1024 * 1024)
    }

    @Test func animatedPhotoPlaysAndAFrameCountOverTheCapUsesTheFirstFrame() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let capped = "Image(\"\(server.url("many-frames"))\", \"Over the frame cap\", 1)"
        let window = try await mountPhotos("root = Col([\(capped), Image(\"\", \"Control\", 1)], 0)",
                                                width: 100, height: 220)
        defer { window.window.closeForTest() }
        let first = try await photoRect("/children/0", in: window)
        try await waitForPhoto(window, in: first)

        // Start the control once the over-cap image has drawn. Both GIFs have
        // a half-second red first frame, so when the control draws blue the
        // over-cap GIF would also be blue if its later frames were drawn.
        try await window.live.show(response: """
        root = Col([\(capped), Image("\(server.url("animated"))", "Control", 1)], 0)
        """, streaming: false)
        let control = try await photoRect("/children/1", in: window)
        try await waitForPhoto(window, in: control)
        try await waitForPhotoEvent("the animated GIF's blue frame", in: window) {
            let pixels = try PhotoWindowPixels(window)
            let blue = try pixels.color(in: control, x: 0.5, y: 0.5)
            guard blue.blueComponent > 0.85, blue.redComponent < 0.15 else { return false }
            #expect(try pixels.color(in: first, x: 0.5, y: 0.5).isPhotoRed)
            return true
        }
        #expect(server.requests == ["/many-frames", "/animated"])
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
        let window = try await mountPhotos("""
        root = Image("\(server.url("vector-covered"))", "Diagram", 2, 0, "contain")
        """, width: 100, height: 70)
        defer { window.window.closeForTest() }
        let photo = try await photoRect("/", in: window)
        try await waitForPhotoEvent("the normalized SVG's red and blue pixels", in: window) {
            let pixels = try PhotoWindowPixels(window)
            let left = try pixels.color(in: photo, x: 0.15, y: 0.5)
            let right = try pixels.color(in: photo, x: 0.30, y: 0.5)
            return left.isPhotoRed && right.blueComponent > 0.85 && right.redComponent < 0.15
        }
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
        let window = try await mountPhotos("""
        root = Col([Image("\(server.url(path))", " Unsupported diagram ", 2, 0),
                    Image("", "Empty reference", 2, 0)], 0)
        """, width: 100, height: 110)
        defer { window.window.closeForTest() }
        let photo = try await photoRect("/children/0", in: window)
        let empty = try await photoRect("/children/1", in: window)
        try await waitForPhotoEvent("the themed empty state for unsupported SVG (\(feature))", in: window) {
            let pixels = try PhotoWindowPixels(window)
            guard try pixels.drawsEmptyGlyph(in: photo), try pixels.drawsEmptyGlyph(in: empty) else { return false }
            let actual = try pixels.color(in: photo, x: 0.2, y: 0.2)
            let reference = try pixels.color(in: empty, x: 0.2, y: 0.2)
            return PhotoWindowPixels.close(actual, reference)
        }
        #expect(imageLabels(in: window).sorted() == ["Empty reference", "Unsupported diagram"])
        #expect(server.requests == ["/" + path])
    }

    @Test func decorativeLoadedImagesAreHiddenButTheEmptyStateRetainsItsImageRole() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("""
        root = Col([Image("\(server.url("decorative"))", "", 2),
                    Image("\(server.url("spaces"))", "   ", 2),
                    Image("\(server.url("named"))", " Named ", 2),
                    Image("\(server.url("broken"))", "", 2)], 0)
        """, width: 100, height: 210)
        defer { window.window.closeForTest() }
        for index in 0..<3 {
            try await waitForPhoto(window, in: photoRect("/children/\(index)", in: window))
        }
        try await waitForPhotoEvent("the failed decorative image's empty image role", in: window) {
            imageLabels(in: window).sorted() == ["", "Named"]
        }
    }

    // MARK: Scrolling away mid-load, and a hidden window

    /// A load cut short by scrolling away is not a failure: scrolling back
    /// requests the photo again and draws it. This pins the guard order in
    /// PhotoContent, where the cancellation and placement checks come before
    /// `onFailure`.
    @Test(arguments: ["SwiftUI", "AppKit"])
    func scrollingAwayMidLoadAndBackDrawsThePicture(_ host: String) async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Text(\"Viewport\")", width: 150, height: 180,
                                           scrollable: host == "SwiftUI")
        defer { window.window.closeForTest() }
        let nativeScroll = host == "AppKit" ? embedInAppKitScrollView(window, documentHeight: 800) : nil
        try await window.live.show(response: """
        root = Col([Image("\(server.url("busy-scroll"))", "Top", 1),
                    Box([], null, null, null, null, null, null, null, 500)], 0)
        """, streaming: false)
        let budget = PhotoBudget.inWindow(window.window)
        try await waitForPhotoEvent("the top photo's held request", in: window) {
            budget.active == 1 && server.metric("peak") == "1"
        }
        let scroll = try #require(nativeScroll ?? window.scrollView)
        scrollTo(scroll, y: 300)
        try await waitForPhotoEvent("the scrolled-away load's slot and bytes to return", in: window) {
            budget.active == 0 && budget.charged == 0
        }
        scrollTo(scroll, y: 0)
        try await waitForPhotoEvent("a second request after scrolling back", in: window) {
            server.requests == ["/busy-scroll", "/busy-scroll"]
        }
        try server.releaseBusyResponses()
        try await waitForPhoto(window, in: try await photoRect("/children/0", in: window))
        try await requirePhotoQuiet("no third request once the photo drew", in: window, server: server,
                                    expected: ["/busy-scroll", "/busy-scroll"], for: .milliseconds(300))
        #expect(budget.active == 0 && (1..<(8 * 1024 * 1024)).contains(budget.charged))
    }

    /// A photo whose load ends while its window is hidden is thrown away, and
    /// requested once more when the window shows again, then drawn.
    @Test(arguments: ["orderOut", "app-hide"])
    func aPhotoLoadedWhileItsWindowIsHiddenDrawsWhenItShows(_ how: String) async throws {
        try await photoLoadedWhileHiddenDrawsWhenItShows(how)
    }

    /// The same with the window minimised. Where AppKit refuses to minimise
    /// a probe window in this process, this case alone skips, naming that.
    @Test(.requiresWindowMinimise)
    func aPhotoLoadedWhileItsWindowIsMinimisedDrawsWhenItShows() async throws {
        try await photoLoadedWhileHiddenDrawsWhenItShows("miniaturize")
    }

    private func photoLoadedWhileHiddenDrawsWhenItShows(_ how: String) async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Text(\"Viewport\")", width: 150, height: 180)
        defer { window.window.closeForTest() }
        try await window.live.show(response: "root = Image(\"\(server.url("busy-hide"))\", \"Photo\", 1)",
                                   streaming: false)
        let budget = PhotoBudget.inWindow(window.window)
        try await waitForPhotoEvent("the photo's held request", in: window) {
            budget.active == 1 && server.metric("peak") == "1"
        }
        let photo = try await photoRect("/", in: window)
        try await hide(window, how)
        try server.releaseBusyResponses()
        try await waitForPhotoEvent("the hidden photo's load to end", in: window) {
            budget.active == 0 && budget.queued == 0
        }
        try await show(window, how)
        try await waitForPhoto(window, in: photo)
        try await requirePhotoQuiet("exactly one more request after showing the window", in: window,
                                    server: server, expected: ["/busy-hide", "/busy-hide"], for: .milliseconds(300))
        #expect(budget.active == 0 && budget.queued == 0)
    }

    /// Hiding and showing the window after a photo drew keeps it drawn and
    /// requests nothing, when nothing updates the view while it is hidden.
    /// An update while hidden (a streamed program update, or tight hide/show
    /// cycles) refreshes the probe, which releases the raster, so showing the
    /// window downloads the photo again, once per show (#250).
    @Test(arguments: ["orderOut", "app-hide"])
    func aDrawnPhotoStaysDrawnWithoutARequestWhenItsWindowHidesAndShows(_ how: String) async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Text(\"Viewport\")", width: 150, height: 180)
        defer { window.window.closeForTest() }
        try await window.live.show(response: "root = Image(\"\(server.url("square"))\", \"Photo\", 1)",
                                   streaming: false)
        let photo = try await photoRect("/", in: window)
        try await waitForPhoto(window, in: photo)
        try await hide(window, how)
        try await show(window, how)
        try await requirePhotoQuiet("no request after hiding and showing a drawn photo", in: window,
                                    server: server, expected: ["/square"], for: .milliseconds(300))
        try await waitForPhoto(window, in: photo)
    }

    /// Four photos load and two wait while the window is hidden. The four
    /// finished loads are thrown away and the two waiters are woken with
    /// nothing; showing the window requests the four again and the two for
    /// the first time, and all six draw.
    @Test(arguments: ["orderOut", "app-hide"])
    func loadingAndQueuedPhotosOfAHiddenWindowAllDrawWhenItShows(_ how: String) async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let paths = (0..<6).map { "/busy-queue-\($0)" }
        let photos = paths.map { "Image(\"\(server.url(String($0.dropFirst())))\", \"Photo\", 4)" }
        let window = try await mountPhotos("root = Col([\(photos.joined(separator: ","))], 0)",
                                           width: 80, height: 260)
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        try await waitForPhotoEvent("four held downloads and two waiters", in: window) {
            budget.active == 4 && budget.queued == 2 && server.metric("peak") == "4"
        }
        let loading = Set(server.requests)
        try #require(loading.count == 4)
        var rects: [CGRect] = []
        for index in paths.indices { rects.append(try await photoRect("/children/\(index)", in: window)) }
        try await hide(window, how)
        try server.releaseBusyResponses()
        try await waitForPhotoEvent("the hidden window's loads to end and its waiters to be dropped", in: window) {
            budget.active == 0 && budget.queued == 0
        }
        #expect(server.requests.count == 4, "A waiter woken while the window is hidden must not load")
        try await show(window, how)
        for rect in rects { try await waitForPhoto(window, in: rect) }
        let expected = paths.flatMap { loading.contains($0) ? [$0, $0] : [$0] }.sorted()
        try await requirePhotoQuiet("each loading photo requested twice and each waiter once", in: window,
                                    server: server, expected: expected, sorted: true, for: .milliseconds(300))
        #expect(budget.active == 0 && budget.queued == 0)
    }

    /// The app in the background with another window in front of this one:
    /// the window stays visible, so the load that ends meanwhile draws, and
    /// nothing is requested again.
    @Test func aPhotoLoadedWhileAnotherWindowIsInFrontDrawsWithoutAnotherRequest() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("root = Text(\"Viewport\")", width: 150, height: 180)
        defer { window.window.closeForTest() }
        try await window.live.show(response: "root = Image(\"\(server.url("busy-behind"))\", \"Photo\", 1)",
                                   streaming: false)
        let budget = PhotoBudget.inWindow(window.window)
        try await waitForPhotoEvent("the photo's held request", in: window) {
            budget.active == 1 && server.metric("peak") == "1"
        }
        let photo = try await photoRect("/", in: window)
        let cover = NSWindow(contentRect: window.window.frame.insetBy(dx: -20, dy: -20), styleMask: [.titled],
                             backing: .buffered, defer: false)
        cover.isReleasedWhenClosed = false
        defer { cover.closeForTest() }
        cover.orderFrontRegardless()
        NSApp.deactivate()
        try await waitForPhotoEvent("the cover window in front", in: window) {
            NSApp.orderedWindows.firstIndex(of: cover).map { index in
                NSApp.orderedWindows.firstIndex(of: window.window).map { index < $0 } ?? false
            } ?? false
        }
        #expect(window.window.isVisible)
        try server.releaseBusyResponses()
        try await waitForPhoto(window, in: photo)
        #expect(server.requests == ["/busy-behind"])
        cover.orderOut(nil)
        window.window.orderFrontRegardless()
        try await requirePhotoQuiet("no second request once the window is in front again", in: window,
                                    server: server, expected: ["/busy-behind"], for: .milliseconds(300))
        try await waitForPhoto(window, in: photo)
    }

    private func hide(_ window: CollectionWindow, _ how: String) async throws {
        switch how {
        case "app-hide":
            NSApp.hide(nil)
        case "miniaturize":
            window.window.styleMask.insert(.miniaturizable)
            window.window.miniaturize(nil)
        default:
            window.window.orderOut(nil)
        }
        try await waitForPhotoEvent("the window to be hidden by \(how)", in: window) {
            !window.window.isVisible && (how != "miniaturize" || window.window.isMiniaturized)
        }
        if how == "miniaturize" {
            try #require(window.window.isMiniaturized, "The window must be minimised before the test goes on")
        }
    }

    private func show(_ window: CollectionWindow, _ how: String) async throws {
        switch how {
        case "app-hide": NSApp.unhide(nil)
        case "miniaturize": window.window.deminiaturize(nil)
        default: window.window.orderFrontRegardless()
        }
        try await waitForPhotoEvent("the window to show again after \(how)", in: window) {
            window.window.isVisible && !window.window.isMiniaturized
        }
    }

    private func waitForPhoto(_ window: CollectionWindow, in photo: CGRect) async throws {
        try await waitForPhotoEvent("the photo's red pixels in \(photo)", in: window) {
            try PhotoWindowPixels(window).color(in: photo, x: 0.25, y: 0.5).isPhotoRed
        }
    }

    private func waitForOpaquePhoto(_ window: CollectionWindow, in photo: CGRect) async throws {
        let opaqueRed = NSColor(srgbRed: 1, green: 0, blue: 0, alpha: 1)
        try await waitForPhotoEvent("the photo's fully opaque red pixels in \(photo)", in: window) {
            PhotoWindowPixels.close(try PhotoWindowPixels(window).color(in: photo, x: 0.25, y: 0.5), opaqueRed)
        }
    }

    private func mountPhotos(_ program: String, width: CGFloat, height: CGFloat,
                             scrollable: Bool = false) async throws -> CollectionWindow {
        try await CollectionWindow(program, width: width, height: height, scrollable: scrollable,
                                   observeProgramDrawing: true)
    }

    private func photoRect(_ path: String, in window: CollectionWindow) async throws -> CGRect {
        try await waitForPhotoEvent("the layout of \(path)", in: window) { window.frame(at: path) != nil }
        return try window.rect(path)
    }

    private func waitForPhotoEvent(_ event: String, in window: CollectionWindow,
                                   untilObserved: () throws -> Bool) async throws {
        try await waitForCollectionEvent(event) {
            window.host.layoutSubtreeIfNeeded()
            return try untilObserved()
        }
    }

    private func requirePhotoQuiet(_ event: String, in window: CollectionWindow, server: PhotoLoopback,
                                   expected: [String], sorted: Bool = false, for observation: Duration) async throws {
        // A stable drawing and a quiet server log cannot rule out a delayed
        // start while the view remains mounted. Observe the original interval
        // and fail early if an unexpected request arrives.
        let end = ContinuousClock.now + observation
        try await waitForPhotoEvent(event, in: window) {
            try #require((sorted ? server.requests.sorted() : server.requests) == expected)
            return ContinuousClock.now >= end
        }
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
        let window = try await mountPhotos("""
        root = Col([Image("\(relative)", "Refused", 2),
                    Box([], null, null, null, null, null, null, null, 250),
                    Image("\(server.url("later"))", "Later", 2)], 0)
        """, width: 180, height: 110, scrollable: true)
        defer { window.window.closeForTest() }
        let photo = try await photoRect("/children/2", in: window)
        #expect(photo.minY >= window.host.bounds.height)
        try await requirePhotoQuiet("no request for the offscreen or refused image", in: window,
                                    server: server, expected: [], for: .milliseconds(100))
        let scroll = try #require(window.scrollView)
        let clip = scroll.contentView
        clip.scroll(to: CGPoint(x: 0, y: photo.minY))
        scroll.reflectScrolledClipView(clip)
        try await waitForPhotoEvent("the image request after scrolling into view", in: window) {
            server.requests.contains("/later")
        }
        #expect(server.requests == ["/later"])
    }

    @Test func loopbackPhotoDrawsCoverAndContainAndFailureDrawsTheEmptyState() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("""
        root = Col([Image("\(server.url("cover"))", " Cover ", 1, 12),
                    Image("\(server.url("contain"))", "Contain", 1, 0, "contain"),
                    Image("\(server.url("broken"))", " Missing ", 1, 0),
                    Image("", "Empty", 1, 0)], 0)
        """, width: 120, height: 490)
        defer { window.window.closeForTest() }
        let cover = try await photoRect("/children/0", in: window)
        let contain = try await photoRect("/children/1", in: window)
        let failed = try await photoRect("/children/2", in: window)
        let empty = try await photoRect("/children/3", in: window)
        try await waitForPhotoEvent("the cover, contain and failed image pixels", in: window) {
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
                return true
            }
            return false
        }
        #expect(server.requests.sorted() == ["/broken", "/contain", "/cover"])
        #expect(imageLabels(in: window).sorted() == ["Contain", "Cover", "Empty", "Missing"])
    }

    @Test func replacingAnImageSourceLoadsTheNewPhoto() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await mountPhotos("""
        root = Col([Image("\(server.url("stalled"))", "Old", 1),
                    Image("", "Empty reference", 1)], 0)
        """, width: 100, height: 220)
        defer { window.window.closeForTest() }
        let rect = try await photoRect("/children/0", in: window)
        let reference = try await photoRect("/children/1", in: window)
        let budget = PhotoBudget.inWindow(window.window)
        try await waitForPhotoEvent("the held request and a rendered empty-glyph reference", in: window) {
            guard server.metric("stalled") == "1", budget.active == 1 else { return false }
            let pixels = try PhotoWindowPixels(window)
            guard let match = try pixels.emptyGlyphMatch(in: rect, reference: reference) else { return false }
            // A stalled load is still loading. A capture that shows the empty
            // glyph here fails the test.
            #expect(match < 0.5, "The stalled photo drew the empty glyph (\(Int(match * 100))% of its pixels)")
            return true
        }
        try #require(server.requests == ["/stalled"])
        // The stalled photo draws the moving shimmer, not a blank or still
        // frame: twelve captures 120 ms apart must differ.
        var shimmer = Set<String>()
        for _ in 0..<12 {
            let pixels = try PhotoWindowPixels(window)
            for x in stride(from: 0.1, through: 0.9, by: 0.2) {
                let color = try pixels.color(in: rect, x: x, y: 0.3)
                shimmer.insert(String(format: "%.2f,%.2f,%.2f", color.redComponent, color.greenComponent,
                                      color.blueComponent))
            }
            try await Task.sleep(for: .milliseconds(120))
        }
        #expect(shimmer.count > 1, "The stalled photo drew one colour: \(shimmer)")
        #expect(server.metric("stalled") == "1")

        try await window.live.show(response: """
        root = Col([Image("\(server.url("new"))", "New", 1),
                    Image("", "Empty reference", 1)], 0)
        """, streaming: false)
        try await waitForPhotoEvent("the replacement image's blue pixels", in: window) {
            let color = try PhotoWindowPixels(window).color(in: rect, x: 0.75, y: 0.5)
            return color.blueComponent > 0.9 && color.redComponent < 0.1
        }
        try await waitForPhotoEvent("the replaced source's connection to close", in: window) {
            server.metric("stalled") == "0"
        }
        #expect(server.requests == ["/stalled", "/new"])
        #expect(imageLabels(in: window).sorted() == ["Empty reference", "New"])
    }

    @Test func returningToTheMostRecentlyFailedSourceShowsTheEmptyStateWithoutRetrying() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let failed = server.url("broken")
        let window = try await mountPhotos("""
        $source = "\(failed)"
        root = Col([Image($source, "Photo", 1), Image("", "Empty reference", 1)], 0)
        """, width: 100, height: 220)
        defer { window.window.closeForTest() }
        let photo = try await photoRect("/children/0", in: window)
        try await waitForPhotoEvent("the failed source's empty glyph", in: window) {
            try PhotoWindowPixels(window).drawsEmptyGlyph(in: photo)
        }
        try #require(server.requests == ["/broken"])

        try window.live.set("$source", to: .string(server.url("new")))
        try await waitForPhotoEvent("the new source's blue pixels", in: window) {
            let color = try PhotoWindowPixels(window).color(in: photo, x: 0.75, y: 0.5)
            return color.blueComponent > 0.9 && color.redComponent < 0.1
        }

        try window.live.set("$source", to: .string(failed))
        try await window.waitForProgramDrawing()
        try await waitForPhotoEvent("the empty state when revisiting the failed source", in: window) {
            try PhotoWindowPixels(window).drawsEmptyGlyph(in: photo)
        }
        try await requirePhotoQuiet("no retry after returning to the failed source", in: window,
                                    server: server, expected: ["/broken", "/new"], for: .milliseconds(300))
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

private extension NSColor {
    var isPhotoRed: Bool { redComponent > 0.85 && blueComponent < 0.15 }
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

@MainActor private final class ReuseOutcomes {
    var values: [Bool] = []
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
        return try #require(bitmap.sRGBColor(x: column, y: row))
    }

    static func close(_ lhs: NSColor, _ rhs: NSColor) -> Bool {
        abs(lhs.redComponent - rhs.redComponent) < 0.04 &&
        abs(lhs.greenComponent - rhs.greenComponent) < 0.04 &&
        abs(lhs.blueComponent - rhs.blueComponent) < 0.04 &&
        abs(lhs.alphaComponent - rhs.alphaComponent) < 0.04
    }

    /// The share of a real empty image's glyph pixels that `rect` reproduces,
    /// or nil until `reference` shows a glyph. Both photos have the same size,
    /// so pixels are compared at the same whole-pixel offset from each centre,
    /// allowing one pixel of rounding either way. Loading never draws the
    /// glyph, so a gradient can match only a few of its pixels.
    func emptyGlyphMatch(in rect: CGRect, reference: CGRect) throws -> Double? {
        let scaleX = CGFloat(bitmap.pixelsWide) / bounds.width
        let scaleY = CGFloat(bitmap.pixelsHigh) / bounds.height
        func center(_ rect: CGRect) -> (x: Int, y: Int) {
            (Int((rect.midX * scaleX).rounded()), Int((rect.midY * scaleY).rounded()))
        }
        func pixel(_ x: Int, _ y: Int) throws -> NSColor {
            try #require(bitmap.sRGBColor(x: x, y: y))
        }
        // The glyph is 26 points square. Sample 30 so the corner is background.
        let reach = Int((15 * scaleX).rounded())
        let actualCenter = center(rect), referenceCenter = center(reference)
        let background = try pixel(referenceCenter.x - reach, referenceCenter.y - reach)
        var glyph: [(dx: Int, dy: Int, color: NSColor)] = []
        for dy in -reach...reach {
            for dx in -reach...reach {
                let color = try pixel(referenceCenter.x + dx, referenceCenter.y + dy)
                if !Self.close(color, background) { glyph.append((dx, dy, color)) }
            }
        }
        guard glyph.count >= 20 else { return nil }
        var best = 0
        for shiftY in -1...1 {
            for shiftX in -1...1 {
                var matches = 0
                for point in glyph {
                    let actual = try pixel(actualCenter.x + point.dx + shiftX, actualCenter.y + point.dy + shiftY)
                    if Self.close(actual, point.color) { matches += 1 }
                }
                best = max(best, matches)
            }
        }
        return Double(best) / Double(glyph.count)
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
