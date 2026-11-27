import AppKit
import SwiftUI
import Testing

extension CollectionDrawingTests {
@Suite @MainActor struct CarouselDrawingTests {
    private func strip(_ props: String = "", width: CGFloat = 320) async throws -> CollectionWindow {
        try await CollectionWindow("""
        root = Col([Carousel([tile, tile, tile, tile]\(props))], 0)
        tile = Box([], null, null, null, "blue", null, null, null, 50)
        """, width: width)
    }

    @Test func carouselUsesItemWidthFourPointGapsAndTheViewportCap() async throws {
        for (props, viewport, width, gap) in [("", 320, 240, 12), (", 12, 2", 320, 80, 8),
                                               (", 800, -1", 800, 640, 0), (", 640, 1", 200, 170, 4)]
            as [(String, CGFloat, CGFloat, CGFloat)] {
            let mounted = try await strip(props, width: viewport)
            defer { mounted.window.close() }
            let first = try mounted.rect("/children/0/children/0")
            let second = try mounted.rect("/children/0/children/1")
            #expect(first.width == width)
            #expect(second.minX - first.maxX == gap)
            #expect(second.minY == first.minY)
        }
    }

    @Test func singleCarouselItemCapsItsGridAt85PercentOfTheTrackWidth() async throws {
        for (alignment, outerWidth) in [("stretch", 320), ("center", 240)] {
            let mounted = try await CollectionWindow("""
            root = Col([Carousel([Grid([a,a], 2, 110, 0)], 240, 0)], 0, "\(alignment)")
            a = Box([], null, null, null, "red", null, null, null, 10)
            """, width: 320)
            defer { mounted.window.close() }
            let grid = "/children/0/children/0"
            #expect(try mounted.rect("/children/0").width == CGFloat(outerWidth))
            #expect(try mounted.rect(grid).width == 204)
            let first = try mounted.rect(grid + "/children/0")
            let second = try mounted.rect(grid + "/children/1")
            #expect(second.minY == first.maxY)
            #expect(second.minX == first.minX)
        }
    }

    @Test func overflowingItemsExtendTheScrollViewButArrowsOnlyTargetItems() async throws {
        let repetitions = carouselRepetitions("OPENUI_REPEAT_CAROUSEL_OVERFLOW")
        for attempt in 1...repetitions {
            try await checkOverflowingItems(attempt: attempt, repetitions: repetitions)
        }
    }

    private func checkOverflowingItems(attempt: Int, repetitions: Int) async throws {
        for (items, count, extent) in [("wide", 1, 360), ("tile, wide", 2, 480), ("wide, tile", 2, 360)] {
            let mounted = try await CollectionWindow("""
            root = Col([Carousel([\(items)], 120, 0)], 0)
            tile = Box([], null, null, null, "blue", null, null, null, 50)
            wide = Box([], null, null, null, "red", null, null, 360, 50)
            """, width: 320)
            defer { mounted.window.close() }
            let clip = try #require(mounted.scrollView?.contentView)
            let lastEdge = CGFloat((count - 1) * 120)
            #expect(clip.bounds.width == CGFloat(count * 120))
            #expect(clip.documentRect.width == CGFloat(extent))
            #expect(mounted.buttonLabels.contains("Next"))
            for _ in 0..<2 {
                try await mounted.click(CGPoint(x: 295, y: 25))
                try await mounted.waitForLayout(for: .seconds(2)) { abs(clip.bounds.minX - lastEdge) <= 1 }
                #expect(abs(clip.bounds.minX - lastEdge) <= 1)
                #expect(mounted.buttonLabels.contains("Next") == (lastEdge + clip.bounds.width < CGFloat(extent)))
            }
            // A manual scroll can put every item origin left of the viewport.
            let endOffset = clip.documentRect.width - clip.bounds.width
            clip.scroll(to: CGPoint(x: endOffset, y: 0))
            // The clip can reach the end before SwiftUI updates its arrow overlay.
            _ = try await waitForScroll {
                mounted.host.layoutSubtreeIfNeeded()
                return abs(clip.bounds.minX - endOffset) <= 1 && !mounted.buttonLabels.contains("Next")
            }
            let actualOffset = clip.bounds.minX
            let nextShown = mounted.buttonLabels.contains("Next")
            let result = "Carousel overflow \(attempt)/\(repetitions) [\(items)]: offset \(actualOffset)/\(endOffset) pt, Next \(nextShown ? "shown" : "hidden")"
            #expect(abs(actualOffset - endOffset) <= 1, "\(result)")
            #expect(!nextShown, "\(result)")
            if repetitions > 1 { print(result) }
            try await mounted.click(CGPoint(x: 25, y: 25))
            try await mounted.waitForLayout(for: .seconds(2)) { abs(clip.bounds.minX) <= 1 }
            #expect(abs(clip.bounds.minX) <= 1)
        }
    }

    @Test func carouselMeasuresDescendantsUntilANestedContainerContainsTheirScrollOverflow() async throws {
        for (item, extent) in [
            ("Col([Grid([wide], 1, null, 0)], 0)", 360),
            (#"Box([wide], null, 0, 0, null, "sm")"#, 120),
            ("Carousel([wide], 120, 0)", 120),
        ] {
            let mounted = try await CollectionWindow("""
            root = Col([Carousel([\(item)], 120, 0)], 0)
            wide = Box([], null, null, null, "red", null, null, 360, 50)
            """, width: 320)
            defer { mounted.window.close() }
            let outer = try #require(mounted.scrollView?.contentView)
            #expect(outer.bounds.width == 120)
            #expect(outer.documentRect.width == CGFloat(extent))
        }
    }

    @Test func gridStretchesCarouselAndCentersItsArrowBesideTheTallCell() async throws {
        let mounted = try await CollectionWindow("""
        root = Col([Grid([Carousel([tile,tile,tile], 80, 1), tall], 2, null, 0)], 0)
        tile = Box([], null, null, null, "blue", null, null, null, 10)
        tall = Box([], null, null, null, "red", null, null, null, 100)
        """, width: 240)
        defer { mounted.window.close() }
        let path = "/children/0/children/0"
        #expect(try mounted.rect(path).size == CGSize(width: 120, height: 100))
        #expect(try mounted.rect(path + "/children/0").minY == 0)
        try await mounted.click(CGPoint(x: 95, y: 5))
        #expect(try mounted.rect(path + "/children/1").minX == 84)
        try await mounted.click(CGPoint(x: 95, y: 50))
        try await mounted.waitForLayout(for: .seconds(2)) { abs(try mounted.rect(path + "/children/1").minX) <= 1 }
        #expect(abs(try mounted.rect(path + "/children/1").minX) <= 1)
    }

    @Test func carouselArrowClicksAdvanceOneItemAndStopAtTheEnds() async throws {
        let mounted = try await strip(", 140, 2")
        defer { mounted.window.close() }
        let track = try mounted.rect("/children/0")
        let next = CGPoint(x: track.maxX - 25, y: track.midY)
        let previous = CGPoint(x: track.minX + 25, y: track.midY)
        func offset() throws -> CGFloat { try track.minX - mounted.rect("/children/0/children/0").minX }
        for (point, target) in [(previous, 0), (next, 148), (next, 264), (next, 264), (previous, 148), (previous, 0)] {
            try await mounted.click(point)
            try await mounted.waitForLayout(for: .seconds(2)) { abs(try offset() - CGFloat(target)) <= 1 }
            try await readWithinFrameBound(mounted) {
                abs(try offset() - CGFloat(target)) <= 0.01 && mounted.buttonLabels.contains("Previous") == (target > 0)
            }
            let actual = try offset()
            #expect(abs(actual - CGFloat(target)) <= 1)
            let labels = Set(mounted.buttonLabels)
            #expect(labels.contains("Previous") == (target > 0))
            #expect(labels.contains("Next") == (target < 264))
        }
    }

    @Test func fractionalCarouselStrideReturnsToTheFirstItemAfterThreeNextClicks() async throws {
        let mounted = try await CollectionWindow("""
        root = Col([Carousel([a,a,a,a,a,a,a,a], 90.17, 0)], 0)
        a = Box([], null, null, null, "blue", null, null, null, 50)
        """, width: 240)
        defer { mounted.window.close() }
        let track = try mounted.rect("/children/0")
        for (index, item) in [1, 2, 3, 2, 1, 0].enumerated() {
            let x = index < 3 ? track.maxX - 25 : track.minX + 25
            try await mounted.click(CGPoint(x: x, y: track.midY))
            let path = "/children/0/children/\(item)"
            try await mounted.waitForLayout(for: .seconds(2)) { abs(try mounted.rect(path).minX - track.minX) <= 1 }
            try await readWithinFrameBound(mounted) { mounted.buttonLabels.contains("Previous") == (item > 0) }
            #expect(abs(try mounted.rect(path).minX - track.minX) <= 1)
        }
        #expect(!mounted.buttonLabels.contains("Previous"))
    }

    @Test func centeredCarouselFitsItsStripWhileStretchedCarouselScrolls() async throws {
        for centered in [true, false] {
            let mounted = try await CollectionWindow("""
            root = Col([Carousel([a,a,a], 120, 1)], 0, "\(centered ? "center" : "stretch")")
            a = Box([], null, null, null, "blue", null, null, null, 50)
            """, width: 240)
            defer { mounted.window.close() }
            let track = try mounted.rect("/children/0")
            #expect(track.width == (centered ? 368 : 240))
            #expect(track.midX == 120)
            #expect(try mounted.rect("/children/0/children/0").minX == track.minX)
            #expect(!mounted.buttonLabels.contains("Previous"))
            #expect(mounted.buttonLabels.contains("Next") == !centered)
        }
    }

    @Test(.requiresDisplay) func horizontalSwipeMovesTheCarouselToAnotherItem() async throws {
        let repetitions = carouselRepetitions("OPENUI_REPEAT_CAROUSEL_SWIPE")
        for attempt in 1...repetitions {
            try await swipeCarousel(attempt: attempt, repetitions: repetitions)
        }
    }

    private func swipeCarousel(attempt: Int, repetitions: Int) async throws {
        // Fit two items and their gap so the trailing scroll limit also aligns an item.
        let mounted = try await strip(", 140, 2", width: 288)
        defer { mounted.window.close() }
        let track = try mounted.rect("/children/0")
        let firstItem = try mounted.rect("/children/0/children/0")
        try #require(abs(firstItem.minX - track.minX) <= 1)
        let laterItems = try (1..<4).map { index in
            let path = "/children/0/children/\(index)"
            return (number: index + 1, path: path, edge: try mounted.rect(path).minX - track.minX)
        }
        let secondItem = try #require(laterItems.first)
        try #require(secondItem.edge > 1)
        let clip = try #require(mounted.scrollView?.contentView)
        let initialOffset = clip.bounds.minX

        // Start just inside item two. The track midpoint is now in the gap.
        let startPoint = CGPoint(x: track.minX + secondItem.edge + 12, y: firstItem.midY)
        let point = mounted.host.convert(startPoint, to: nil)
        // Use a fixed gesture duration even when AppKit takes longer to process a sample.
        let timestamp = ProcessInfo.processInfo.systemUptime - 0.1
        func send(phase: Int64, delta: Int64, sample: Int) throws {
            let located = try #require(NSEvent.mouseEvent(with: .mouseMoved, location: point, modifierFlags: [],
                timestamp: timestamp + Double(sample) * 0.025, windowNumber: mounted.window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 0, pressure: 0))
            let event = try #require(located.cgEvent?.copy())
            event.type = .scrollWheel
            event.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
            event.setIntegerValueField(.scrollWheelEventScrollPhase, value: phase)
            event.setIntegerValueField(.scrollWheelEventDeltaAxis2, value: delta.signum())
            event.setIntegerValueField(.scrollWheelEventPointDeltaAxis2, value: delta)
            event.setDoubleValueField(.scrollWheelEventFixedPtDeltaAxis2, value: Double(delta) / 10)
            mounted.window.sendEvent(try #require(NSEvent(cgEvent: event)))
        }

        // Keep the drag distance tied to one measured item stride. The native snap can carry it farther.
        let delta = try #require(Int64(exactly: (-secondItem.edge * 0.24).rounded()))
        try send(phase: 1, delta: delta, sample: 0) // Quartz began
        var lastSample = 0
        var handled = true
        for sample in 1...3 {
            let offset = sample == 1 ? initialOffset : clip.bounds.minX
            try send(phase: 2, delta: delta, sample: sample) // Quartz changed
            lastSample = sample
            handled = try await waitForScroll { clip.bounds.minX > offset }
            if !handled { break }
        }
        let releaseOffset = clip.bounds.minX - initialOffset
        try send(phase: 4, delta: 0, sample: lastSample + 1) // Quartz ended

        var alignedItem: Int?
        var lastOffset = initialOffset
        var stableSince = ContinuousClock.now
        let settled = handled ? try await waitForScroll {
            mounted.host.layoutSubtreeIfNeeded()
            let offset = clip.bounds.minX
            let movement = offset - initialOffset
            let item = try laterItems.first {
                guard movement > 1, abs(movement - $0.edge) <= 1 else { return false }
                return abs(try mounted.rect($0.path).minX - track.minX) <= 1
            }?.number
            let now = ContinuousClock.now
            if item != alignedItem || offset != lastOffset {
                alignedItem = item
                lastOffset = offset
                stableSince = now
            }
            // A snap can pass through an aligned item on its way to a later one.
            return item != nil && now - stableSince >= .milliseconds(100)
        } : false
        let movement = clip.bounds.minX - initialOffset
        let end = try mounted.rect(secondItem.path).minX - track.minX
        let landing = alignedItem.map { "item \($0) aligned" } ?? "no later item aligned"
        let result = "Carousel swipe \(attempt)/\(repetitions): moved \(movement) pt (\(releaseOffset) pt before release), item two ended \(end) pt from the leading edge, \(landing)"
        if repetitions > 1 { print("\(result) (\(settled ? "pass" : "fail"))") }
        #expect(settled, "\(result) (last change sample \(lastSample) advanced: \(handled))")
    }

    // Print to stderr, past the harness's output capture, so a passing CI log
    // records which path the runner's Carousel tests take.
    @Test func runnerDisplayFactsArePrinted() async throws {
        let state = DisplayFrames.state
        let screens = NSScreen.screens.count
        let asleep = screens > 0 && DisplayFrames.mainDisplayIsAsleep
        let ticks = DisplayFrames.ticks(for: .seconds(1))
        let path = DisplayFrames.animates ? "animated" : "not animated"
        let line = "[display-facts] NSScreen.screens: \(screens); main display asleep: \(asleep); "
            + "display-link ticks in 1 s: \(ticks); Carousel tests path: \(path) (\(state))\n"
        try FileHandle.standardError.write(contentsOf: Data(line.utf8))
        #expect((state == .absent) == (screens == 0))
        if asleep { #expect(state == .asleep) }
    }

    // Runs the animated scroll path only where display frames tick. The arrow
    // reads its current item from the scroll geometry, so a stale offset after
    // an animation would send a quick second click nowhere.
    @Test(.requiresDisplay) func animatedArrowClicksLandOnItemStartsAndQuickClicksAdvanceOneItemEach() async throws {
        #expect(DisplayFrames.animates)
        // An arrow click lands exactly on each item's start.
        do {
            let mounted = try await strip(", 140, 2")
            defer { mounted.window.close() }
            let track = try mounted.rect("/children/0")
            let clip = try #require(mounted.scrollView?.contentView)
            let next = CGPoint(x: track.maxX - 25, y: track.midY)
            let previous = CGPoint(x: track.minX + 25, y: track.midY)
            func offset() throws -> CGFloat { try track.minX - mounted.rect("/children/0/children/0").minX }
            for (step, (point, target)) in [(previous, 0), (next, 148), (next, 264), (next, 264), (previous, 148),
                                            (previous, 0)].enumerated() {
                try await mounted.click(point)
                let settled = try await settledOffset(clip, at: CGFloat(target), in: mounted)
                try await mounted.waitForLayout(for: .seconds(2)) { abs(try offset() - CGFloat(target)) <= 0.01 }
                #expect(settled, "arrow step \(step): clip at \(clip.bounds.minX) pt, expected \(target)")
                let actual = try offset()
                #expect(abs(actual - CGFloat(target)) <= 0.01,
                        "arrow step \(step): first item \(actual) pt left of the track, expected \(target)")
                let labels = Set(mounted.buttonLabels)
                #expect(labels.contains("Previous") == (target > 0))
                #expect(labels.contains("Next") == (target < 264))
            }
        }
        // The fractional stride returns to the first item after three Next clicks.
        do {
            let mounted = try await CollectionWindow("""
            root = Col([Carousel([a,a,a,a,a,a,a,a], 90.17, 0)], 0)
            a = Box([], null, null, null, "blue", null, null, null, 50)
            """, width: 240)
            defer { mounted.window.close() }
            let track = try mounted.rect("/children/0")
            for (index, item) in [1, 2, 3, 2, 1, 0].enumerated() {
                let x = index < 3 ? track.maxX - 25 : track.minX + 25
                try await mounted.click(CGPoint(x: x, y: track.midY))
                let path = "/children/0/children/\(item)"
                // Pixel rounding of the 90.17 pt stride leaves at most half a point.
                try await mounted.waitForLayout(for: .seconds(2)) { abs(try mounted.rect(path).minX - track.minX) <= 0.5 }
                try await readWithinFrameBound(mounted) { mounted.buttonLabels.contains("Previous") == (item > 0) }
                let edge = try mounted.rect(path).minX - track.minX
                #expect(abs(edge) <= 0.5, "fractional click \(index): item \(item) at \(edge) pt")
            }
            #expect(!mounted.buttonLabels.contains("Previous"))
        }
        // Quick consecutive clicks: each Next, sent as soon as the previous
        // animation lands, advances exactly one item.
        do {
            let mounted = try await CollectionWindow("""
            root = Col([Carousel([tile, tile, tile, tile, tile, tile], 140, 2)], 0)
            tile = Box([], null, null, null, "blue", null, null, null, 50)
            """, width: 320)
            defer { mounted.window.close() }
            let track = try mounted.rect("/children/0")
            let clip = try #require(mounted.scrollView?.contentView)
            let next = CGPoint(x: track.maxX - 25, y: track.midY)
            // Item starts every 148 pt; the last stop is the scroll limit, 6 × 140 + 5 × 8 − 320.
            for (click, target) in [148, 296, 444, 560].enumerated() {
                try await mounted.click(next)
                let landed = try await landedOffset(clip, at: CGFloat(target))
                #expect(landed, "quick click \(click + 1): clip at \(clip.bounds.minX) pt, expected \(target)")
                if !landed { break }
            }
            #expect(try await settledOffset(clip, at: 560, in: mounted))
            #expect(!mounted.buttonLabels.contains("Next"))
        }
    }

    /// A 1 pt or 0.5 pt wait can pass one frame before the clip reaches its
    /// target: at 120 Hz the last step before an item start is 0.5 pt. The
    /// arrows follow the clip on that next frame. On an awake 120 Hz display
    /// the worst gap measured was 7.7 ms, one frame. 50 ms covers that with
    /// room for a slow or 60 Hz frame (16.7 ms). Wait until the settled
    /// state holds or the bound passes, then let the caller's checks read.
    /// The tolerances stay as they are.
    private func readWithinFrameBound(_ mounted: CollectionWindow, until settled: () throws -> Bool) async throws {
        try await mounted.waitForLayout(for: .milliseconds(50), until: settled)
    }

    /// Poll the clip every few milliseconds, faster than a frame, and return as
    /// soon as it reaches the target: the moment the animation lands.
    private func landedOffset(_ clip: NSClipView, at target: CGFloat) async throws -> Bool {
        let deadline = ContinuousClock.now + .seconds(2)
        while abs(clip.bounds.minX - target) > 0.01 {
            guard ContinuousClock.now < deadline else { return false }
            try await Task.sleep(for: .milliseconds(2))
        }
        return true
    }

    /// The clip reaches the target and stays there for 100 ms.
    private func settledOffset(_ clip: NSClipView, at target: CGFloat, in mounted: CollectionWindow) async throws -> Bool {
        guard try await landedOffset(clip, at: target) else { return false }
        try await Task.sleep(for: .milliseconds(100))
        await mounted.settle()
        return abs(clip.bounds.minX - target) <= 0.01
    }

    private func carouselRepetitions(_ variable: String) -> Int {
        max(1, ProcessInfo.processInfo.environment[variable].flatMap(Int.init) ?? 1)
    }

    private func waitForScroll(_ condition: () throws -> Bool) async throws -> Bool {
        let deadline = ContinuousClock.now + .seconds(3)
        while try !condition() {
            guard ContinuousClock.now < deadline else { return false }
            try await Task.sleep(for: .milliseconds(10))
        }
        return true
    }

    @Test func carouselKeepsEachAndScalarItemsDistinctAndStretchesCardsToTheTallest() async throws {
        let mounted = try await CollectionWindow("""
        root = Col([Carousel(["one", "two", null, @Each([20, 60], "h", Card([Box([], null, null, null, "red", null, null, null, h)], "outline", 0, 0)), Col([Divider()])], 90, 1)])
        """)
        defer { mounted.window.close() }
        let first = try mounted.rect("/children/0/children/3/0")
        let second = try mounted.rect("/children/0/children/3/1")
        #expect(first.minX == 188)
        #expect(second.minX == 282)
        #expect(second.height == 62)
        #expect(first.height == second.height)
        #expect(try mounted.rect("/children/0/children/4").height == 1)
    }
}
}
