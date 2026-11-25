import AppKit
import SwiftUI
import SwiftTestSupport
import Testing

// The 400-attempt overflow stress run took 104 s under load.
private let stressRepetitions = ["OPENUI_REPEAT_CAROUSEL_OVERFLOW", "OPENUI_REPEAT_CAROUSEL_SWIPE"].contains {
    (ProcessInfo.processInfo.environment[$0].flatMap(Int.init) ?? 1) > 1
}

extension CollectionDrawingTests {
@Suite(NativeTestWatchdog(seconds: stressRepetitions ? 240 : 150)) @MainActor struct CarouselDrawingTests {
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
            defer { mounted.window.closeForTest() }
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
            defer { mounted.window.closeForTest() }
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
            NativeTestWatchdog.progress()
        }
    }

    private func checkOverflowingItems(attempt: Int, repetitions: Int) async throws {
        for (items, count, extent) in [("wide", 1, 360), ("tile, wide", 2, 480), ("wide, tile", 2, 360)] {
            let mounted = try await CollectionWindow("""
            root = Col([Carousel([\(items)], 120, 0)], 0)
            tile = Box([], null, null, null, "blue", null, null, null, 50)
            wide = Box([], null, null, null, "red", null, null, 360, 50)
            """, width: 320)
            defer { mounted.window.closeForTest() }
            let clip = try #require(mounted.scrollView?.contentView)
            let lastEdge = CGFloat((count - 1) * 120)
            #expect(clip.bounds.width == CGFloat(count * 120))
            #expect(clip.documentRect.width == CGFloat(extent))
            #expect(mounted.buttonLabels.contains("Next"))
            for _ in 0..<2 {
                try await mounted.click(CGPoint(x: 295, y: 25))
                try await mounted.requireLayout("the carousel to reach its last item", for: .seconds(2)) {
                    abs(clip.bounds.minX - lastEdge) <= 1
                }
                #expect(abs(clip.bounds.minX - lastEdge) <= 1)
                #expect(mounted.buttonLabels.contains("Next") == (lastEdge + clip.bounds.width < CGFloat(extent)))
            }
            // A manual scroll can put every item origin left of the viewport.
            let endOffset = clip.documentRect.width - clip.bounds.width
            NativeTestWatchdog.waitingFor("AppKit to scroll to the end of the carousel")
            clip.scroll(to: CGPoint(x: endOffset, y: 0))
            // The clip can reach the end before SwiftUI updates its arrow overlay.
            let reachedEnd = try await waitForScroll("the carousel scroll view to reach its final offset") {
                mounted.host.layoutSubtreeIfNeeded()
                return abs(clip.bounds.minX - endOffset) <= 1 && !mounted.buttonLabels.contains("Next")
            }
            let actualOffset = clip.bounds.minX
            let nextShown = mounted.buttonLabels.contains("Next")
            let result = "Carousel overflow \(attempt)/\(repetitions) [\(items)]: offset \(actualOffset)/\(endOffset) pt, Next \(nextShown ? "shown" : "hidden")"
            #expect(reachedEnd, "Timed out waiting for the carousel to reach its final offset. \(result)")
            #expect(abs(actualOffset - endOffset) <= 1, "\(result)")
            #expect(!nextShown, "\(result)")
            if repetitions > 1 { print(result) }
            try await mounted.click(CGPoint(x: 25, y: 25))
            try await mounted.requireLayout("the carousel to return to its first item", for: .seconds(2)) {
                abs(clip.bounds.minX) <= 1
            }
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
            defer { mounted.window.closeForTest() }
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
        defer { mounted.window.closeForTest() }
        let path = "/children/0/children/0"
        #expect(try mounted.rect(path).size == CGSize(width: 120, height: 100))
        #expect(try mounted.rect(path + "/children/0").minY == 0)
        try await mounted.click(CGPoint(x: 95, y: 5))
        #expect(try mounted.rect(path + "/children/1").minX == 84)
        try await mounted.click(CGPoint(x: 95, y: 50))
        try await mounted.requireLayout("the second item to reach the leading edge", for: .seconds(2)) {
            abs(try mounted.rect(path + "/children/1").minX) <= 1
        }
        #expect(abs(try mounted.rect(path + "/children/1").minX) <= 1)
    }

    @Test func carouselArrowClicksAdvanceOneItemAndStopAtTheEnds() async throws {
        let mounted = try await strip(", 140, 2")
        defer { mounted.window.closeForTest() }
        let track = try mounted.rect("/children/0")
        let next = CGPoint(x: track.maxX - 25, y: track.midY)
        let previous = CGPoint(x: track.minX + 25, y: track.midY)
        func offset() throws -> CGFloat { try track.minX - mounted.rect("/children/0/children/0").minX }
        for (point, target) in [(previous, 0), (next, 148), (next, 264), (next, 264), (previous, 148), (previous, 0)] {
            try await mounted.click(point)
            try await mounted.requireLayout("the carousel to reach offset \(target)", for: .seconds(2)) {
                abs(try offset() - CGFloat(target)) <= 1
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
        defer { mounted.window.closeForTest() }
        let track = try mounted.rect("/children/0")
        for (index, item) in [1, 2, 3, 2, 1, 0].enumerated() {
            let x = index < 3 ? track.maxX - 25 : track.minX + 25
            try await mounted.click(CGPoint(x: x, y: track.midY))
            let path = "/children/0/children/\(item)"
            try await mounted.requireLayout("the updated carousel to align its item with the track", for: .seconds(2)) {
                abs(try mounted.rect(path).minX - track.minX) <= 1
            }
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
            defer { mounted.window.closeForTest() }
            let track = try mounted.rect("/children/0")
            #expect(track.width == (centered ? 368 : 240))
            #expect(track.midX == 120)
            #expect(try mounted.rect("/children/0/children/0").minX == track.minX)
            #expect(!mounted.buttonLabels.contains("Previous"))
            #expect(mounted.buttonLabels.contains("Next") == !centered)
        }
    }

    @Test(.requiresDisplay("horizontalSwipeMovesTheCarouselToAnotherItem()")) func horizontalSwipeMovesTheCarouselToAnotherItem() async throws {
        let repetitions = carouselRepetitions("OPENUI_REPEAT_CAROUSEL_SWIPE")
        for attempt in 1...repetitions {
            try await swipeCarousel(attempt: attempt, repetitions: repetitions)
            NativeTestWatchdog.progress()
        }
    }

    private func swipeCarousel(attempt: Int, repetitions: Int) async throws {
        // Fit two items and their gap so the trailing scroll limit also aligns an item.
        let mounted = try await strip(", 140, 2", width: 288)
        defer { mounted.window.closeForTest() }
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
            NativeTestWatchdog.waitingFor("AppKit to dispatch the carousel scroll wheel event")
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
            handled = try await waitForScroll("the scroll wheel event to move the carousel") { clip.bounds.minX > offset }
            if !handled { break }
        }
        let releaseOffset = clip.bounds.minX - initialOffset
        try send(phase: 4, delta: 0, sample: lastSample + 1) // Quartz ended

        var alignedItem: Int?
        var lastOffset = initialOffset
        var stableSince = ContinuousClock.now
        let settled = handled ? try await waitForScroll("the carousel to settle on a swipe destination") {
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

    private func carouselRepetitions(_ variable: String) -> Int {
        max(1, ProcessInfo.processInfo.environment[variable].flatMap(Int.init) ?? 1)
    }

    private func waitForScroll(_ operation: String, _ condition: () throws -> Bool) async throws -> Bool {
        NativeTestWatchdog.waitingFor(operation)
        defer { NativeTestWatchdog.finishedWaiting() }
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
        defer { mounted.window.closeForTest() }
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
