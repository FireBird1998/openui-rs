import AppKit
@testable import OpenUIIntelligentUI
import Testing

extension CollectionDrawingTests {
    @Suite @MainActor struct PolarChartInteractionTests {
        @Test(arguments: ["pie", "donut"], ["number", "percent"])
        func plotHoverPopsOutTheWedgeDimsOthersAndDrawsTheTooltip(kind: String, format: String) async throws {
            let window = try await CollectionWindow("""
            root = Col([Chart("\(kind)", [{name:"Slice A",value:3},{name:"Slice B",value:1}], null, null, "\(format)")], 0)
            """, width: 200, height: 420)
            defer { window.window.close() }

            let plot = try plotFrame(kind, in: window)
            let center = CGPoint(x: plot.midX, y: plot.midY)
            let other = CGPoint(x: center.x - 82, y: center.y - 25)
            let otherTrack = CGPoint(x: center.x - 51, y: center.y - 24)
            let popped = CGPoint(x: center.x + 67, y: center.y + 67)
            let row = try localFrame(of: button("Hide Slice A", in: window), in: window)
            let rest = try capture(window)
            let colour = try rest.pixel(at: CGPoint(x: row.minX + 24, y: row.midY))
            #expect(colour.alphaComponent > 0.99)
            #expect(try rest.pixel(at: other).alphaComponent > 0.99)
            #expect(try rest.pixel(at: popped).alphaComponent < 0.01)
            #expect(try tooltipText(in: window).isEmpty)

            try await movePointer(to: CGPoint(x: center.x + 61, y: center.y + 61), in: window)
            let hovered = try capture(window)
            #expect(abs(try hovered.pixel(at: other).alphaComponent - 0.4) < 0.02)
            if kind == "donut" {
                let track = try rest.pixel(at: otherTrack).alphaComponent
                #expect(track > 0.02)
                #expect(abs(try hovered.pixel(at: otherTrack).alphaComponent - track * 0.4) < 0.01)
            }
            #expect(try hovered.pixel(at: popped).alphaComponent > 0.99)
            let value = format == "percent" ? "75.0%" : "3"
            let text = try tooltipText(in: window)
            #expect(text.map(\.value).sorted() == ["Slice A", "Slice A", value].sorted())
            let labels = text.filter { $0.value == "Slice A" }.sorted { $0.frame.minY < $1.frame.minY }
            if let heading = labels.first, let item = labels.dropFirst().first,
               let amount = text.first(where: { $0.value == value }) {
                #expect(heading.frame.maxY <= item.frame.minY)
                #expect(item.frame.maxY <= amount.frame.minY)
                #expect(abs(item.frame.minX - amount.frame.minX) <= 1)
                let swatch = CGPoint(x: (heading.frame.minX + item.frame.minX - 4) / 2, y: item.frame.midY)
                let paint = try hovered.pixel(at: swatch)
                #expect(abs(paint.redComponent - colour.redComponent) < 0.05)
                #expect(abs(paint.greenComponent - colour.greenComponent) < 0.05)
                #expect(abs(paint.blueComponent - colour.blueComponent) < 0.05)
            }

            // The visible wedge moved here, but its hit area stayed at rest.
            try await movePointer(to: popped, in: window)
            let cleared = try capture(window)
            #expect(try cleared.pixel(at: popped).alphaComponent < 0.01)
            #expect(try cleared.pixel(at: other).alphaComponent > 0.99)
            #expect(try tooltipText(in: window).isEmpty)
        }

        @Test(arguments: ["pie", "donut"], ["Zero", "Debt"])
        func hoveringANonPositiveLegendRowDimsThePlot(kind: String, label: String) async throws {
            let window = try await CollectionWindow("""
            root = Col([Chart("\(kind)", [{name:"Positive",value:4},{name:"Zero",value:0},{name:"Debt",value:-2}])], 0)
            """, width: 200, height: 420)
            defer { window.window.close() }

            let plot = try plotFrame(kind, in: window)
            let wedge = CGPoint(x: plot.midX + 85, y: plot.midY)
            #expect(try capture(window).pixel(at: wedge).alphaComponent > 0.99)
            let row = try localFrame(of: button("Hide " + label, in: window), in: window)
            try await movePointer(to: CGPoint(x: row.midX, y: row.midY), in: window)
            #expect(abs(try capture(window).pixel(at: wedge).alphaComponent - 0.4) < 0.02)
            #expect(try tooltipText(in: window).isEmpty)

            try await movePointer(to: CGPoint(x: plot.minX + 2, y: plot.minY + 2), in: window)
            #expect(try capture(window).pixel(at: wedge).alphaComponent > 0.99)
        }

        @Test(arguments: ["pie", "donut"])
        func plotHoverUsesTheEffectiveOuterRadius(kind: String) async throws {
            let window = try await CollectionWindow("""
            root = Col([Chart("\(kind)", [{name:"Only",value:1}])], 0)
            """, width: 399, height: 500)
            defer { window.window.close() }

            let chart = try window.rect("/children/0")
            let center = CGPoint(x: chart.midX, y: chart.minY + 150)
            try await movePointer(to: CGPoint(x: center.x + 90, y: center.y), in: window)
            #expect(try !tooltipText(in: window).isEmpty)
            try await movePointer(to: CGPoint(x: center.x + 155, y: center.y), in: window)
            #expect(try tooltipText(in: window).isEmpty)
        }

        @Test func clickingLegendTogglesTheRightRepeatedLabelAndUpdatesShares() async throws {
            let window = try await CollectionWindow("""
            root = Chart("donut", [{name:"A",value:4},{name:"A",value:2},{name:"B",value:2}], null, null, "percent")
            """, width: 400, height: 370)
            defer { window.window.close() }

            let duplicate = window.accessibleButtons.filter { $0.accessibilityLabel?()?.hasPrefix("Hide A, ") == true }
            try #require(duplicate.count == 2)
            #expect(duplicate.compactMap { $0.accessibilityLabel?() }.sorted() == ["Hide A, 25.0%", "Hide A, 50.0%"])
            let a = try #require(duplicate.first)
            let point = try localFrame(of: a, in: window)
            try await window.click(CGPoint(x: point.midX, y: point.midY))
            #expect(window.buttonLabels.filter { $0.hasPrefix("Show A, ") }.count == 2)
            #expect(window.buttonLabels.contains("Hide B, 100.0%"))
            try await window.click(CGPoint(x: point.midX, y: point.midY))
            #expect(window.buttonLabels.filter { $0.hasPrefix("Hide A, ") }.count == 2)
        }

        @Test func narrowLegendExpandsToAllRowsAndCollapses() async throws {
            let window = try await CollectionWindow(program(8), width: 320, height: 700)
            defer { window.window.close() }

            #expect(window.buttonLabels.contains { $0.hasPrefix("Hide Row 6, ") })
            #expect(!window.buttonLabels.contains { $0.hasPrefix("Hide Row 7, ") })
            try await click("Show more", in: window)
            #expect(window.buttonLabels.contains("Show less"))
            #expect(window.buttonLabels.contains { $0.hasPrefix("Hide Row 8, ") })
            try await click("Hide Row 8", in: window)
            #expect(window.buttonLabels.contains { $0.hasPrefix("Show Row 8, ") })
            try await click("Show less", in: window)
            #expect(!window.buttonLabels.contains { $0.hasPrefix("Show Row 8, ") })
            try await click("Show more", in: window)
            #expect(window.buttonLabels.contains { $0.hasPrefix("Show Row 8, ") })
        }

        @Test func sideLegendScrollsOneRowAndRejectsAnOverflowingRowClick() async throws {
            let window = try await CollectionWindow(program(14), width: 440, height: 360)
            defer { window.window.close() }

            let scroll = try #require(window.scrollView)
            let clip = scroll.contentView
            let viewport = clip.convert(clip.bounds, to: window.host)
            let rowHeight = clip.documentRect.height / 14
            #expect(abs(rowHeight - 32) < 1)
            let ninthTop = viewport.minY + 8 * rowHeight
            let outside = CGPoint(x: viewport.midX, y: viewport.maxY + 1)
            try #require(ninthTop < viewport.maxY && outside.y < ninthTop + rowHeight)
            let start = clip.bounds.minY
            try await window.click(outside)
            try await click("Scroll legend down", in: window)
            let moved = clip.bounds.minY - start
            #expect(abs(moved - rowHeight) < 1)
            let inside = CGPoint(x: viewport.midX, y: ninthTop + rowHeight / 2 - moved)
            try #require(viewport.contains(inside))
            try await window.click(inside)
            try await click("Scroll legend up", in: window)
            #expect(abs(clip.bounds.minY - start) < 1)

            // The unscrolled layout exposes all of the legend labels to the
            // mounted accessibility probe, including those below row six.
            window.window.setContentSize(NSSize(width: 320, height: 840))
            await window.settle()
            try await click("Show more", in: window)
            #expect(window.buttonLabels.contains { $0.hasPrefix("Show Row 9, ") })
            #expect(window.buttonLabels.contains { $0.hasPrefix("Hide Row 10, ") })
        }

        private func movePointer(to point: CGPoint, in window: CollectionWindow) async throws {
            let move = try #require(NSEvent.mouseEvent(with: .mouseMoved, location: window.host.convert(point, to: nil),
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 0, pressure: 0))
            // The hosting view owns SwiftUI's tracking area. Deliver directly to that
            // responder so the window server needn't route a hardware mouse move.
            window.host.mouseMoved(with: move)
            await window.settle()
        }

        private func plotFrame(_ kind: String, in window: CollectionWindow) throws -> CGRect {
            let title = kind == "pie" ? "Pie chart" : "Donut chart"
            let node = try #require(accessibilityNodes(in: window).first { $0.accessibilityLabel?() == title })
            return try localFrame(of: node, in: window)
        }

        private func tooltipText(in window: CollectionWindow) throws -> [(value: String, frame: CGRect)] {
            try accessibilityNodes(in: window).compactMap { node in
                let role = node.accessibilityRole?()
                guard role == .staticText || role?.rawValue == "AXHeading" else { return nil }
                let value: String? = node.accessibilityValue?()
                guard let text = value ?? node.accessibilityLabel?() else { return nil }
                return (text, try localFrame(of: node, in: window))
            }
        }

        private func accessibilityNodes(in window: CollectionWindow) -> [AnyObject] {
            var pending: [AnyObject] = [window.host]
            var nodes: [AnyObject] = []
            while let node = pending.popLast() {
                nodes.append(node)
                pending.append(contentsOf: (node.accessibilityChildren?() ?? []).map { $0 as AnyObject })
            }
            return nodes
        }

        private func capture(_ window: CollectionWindow) throws -> MountedPixels {
            let bounds = window.host.bounds
            let bitmap = try #require(window.host.bitmapImageRepForCachingDisplay(in: bounds))
            window.host.cacheDisplay(in: bounds, to: bitmap)
            return MountedPixels(bitmap: bitmap, bounds: bounds)
        }

        private struct MountedPixels {
            let bitmap: NSBitmapImageRep
            let bounds: CGRect

            func pixel(at point: CGPoint) throws -> NSColor {
                let x = Int((point.x - bounds.minX) * CGFloat(bitmap.pixelsWide) / bounds.width)
                let y = Int((point.y - bounds.minY) * CGFloat(bitmap.pixelsHigh) / bounds.height)
                return try #require(bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB))
            }
        }

        private func program(_ count: Int) -> String {
            let rows = (1...count).map { "{name:\"Row \($0)\",value:\(count - $0 + 1)}" }
            return "root = Col([Chart(\"pie\", [\(rows.joined(separator: ","))]), Divider()], 0)"
        }

        private func button(_ label: String, in window: CollectionWindow) throws -> AnyObject {
            try #require(window.accessibleButtons.first {
                let title = $0.accessibilityLabel?() ?? $0.accessibilityTitle?() ?? ""
                return title == label || title.hasPrefix(label + ", ")
            }, "Expected \(label) among \(window.buttonLabels)")
        }

        private func click(_ label: String, in window: CollectionWindow) async throws {
            let rect = try localFrame(of: button(label, in: window), in: window)
            try await window.click(CGPoint(x: rect.midX, y: rect.midY))
        }

        private func localFrame(of button: AnyObject, in window: CollectionWindow) throws -> CGRect {
            let screen = try #require(button.accessibilityFrame?())
            return window.host.convert(window.window.convertFromScreen(screen), from: nil)
        }
    }
}
