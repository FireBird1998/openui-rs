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
                for _ in 0..<10 where abs(clip.bounds.minX - lastEdge) > 1 { await mounted.settle() }
                #expect(abs(clip.bounds.minX - lastEdge) <= 1)
                #expect(mounted.buttonLabels.contains("Next") == (lastEdge + clip.bounds.width < CGFloat(extent)))
            }
            // A manual scroll can put every item origin left of the viewport.
            clip.scroll(to: CGPoint(x: clip.documentRect.width - clip.bounds.width, y: 0))
            await mounted.settle()
            #expect(!mounted.buttonLabels.contains("Next"))
            try await mounted.click(CGPoint(x: 25, y: 25))
            for _ in 0..<10 where abs(clip.bounds.minX) > 1 { await mounted.settle() }
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
        for _ in 0..<10 where abs(try mounted.rect(path + "/children/1").minX) > 1 { await mounted.settle() }
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
            for _ in 0..<15 where abs(try offset() - CGFloat(target)) > 1 { await mounted.settle() }
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
            for _ in 0..<15 where abs(try mounted.rect(path).minX - track.minX) > 1 { await mounted.settle() }
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
        let mounted = try await strip(", 140, 2")
        defer { mounted.window.close() }
        let track = try mounted.rect("/children/0")
        // Reuse AppKit's window and location fields from a located event.
        // A headless window has number zero and cannot receive this sequence.
        let point = mounted.host.convert(CGPoint(x: track.midX, y: track.midY), to: nil)
        for phase in [1, 2, 2, 2, 4] { // Quartz began, changed, ended
            let located = try #require(NSEvent.mouseEvent(with: .mouseMoved, location: point, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: mounted.window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 0, pressure: 0))
            let event = try #require(located.cgEvent?.copy())
            event.type = .scrollWheel
            let delta: Int64 = phase == 4 ? 0 : -36
            event.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
            event.setIntegerValueField(.scrollWheelEventScrollPhase, value: Int64(phase))
            event.setIntegerValueField(.scrollWheelEventDeltaAxis2, value: delta.signum())
            event.setIntegerValueField(.scrollWheelEventPointDeltaAxis2, value: delta)
            event.setDoubleValueField(.scrollWheelEventFixedPtDeltaAxis2, value: Double(delta) / 10)
            mounted.window.sendEvent(try #require(NSEvent(cgEvent: event)))
            try await Task.sleep(for: .milliseconds(25))
        }
        for _ in 0..<15 where abs(try mounted.rect("/children/0/children/1").minX - track.minX) > 1 {
            await mounted.settle()
        }
        let end = try mounted.rect("/children/0/children/1").minX
        #expect(abs(end - track.minX) <= 1)
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
