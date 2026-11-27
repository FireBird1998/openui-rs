import AppKit
@testable import OpenUIIntelligentUI
import SwiftUI
import Testing

extension CollectionDrawingTests {
    @Suite @MainActor struct PolarChartInteractionTests {
        @Test(arguments: ["pie", "donut"],
              ["number", "percent"].flatMap { format in WindowColorSpace.allCases.map { (format, $0) } })
        func plotHoverPopsOutTheWedgeDimsOthersAndDrawsTheTooltip(kind: String, variant: (String, WindowColorSpace)) async throws {
            let (format, space) = variant
            let hover = PolarChartHoverInput()
            let window = try await mount("""
            root = Col([Chart("\(kind)", [{name:"Slice A",value:3},{name:"Slice B",value:1}], null, null, "\(format)")], 0)
            """, kind: kind, hover: hover, colorSpace: space)
            defer { window.window.close() }

            let plot = try plotFrame(kind, in: window)
            let center = CGPoint(x: plot.midX, y: plot.midY)
            let other = CGPoint(x: center.x - 82, y: center.y - 25)
            let otherTrack = CGPoint(x: center.x - 51, y: center.y - 24)
            let popped = CGPoint(x: center.x + 67, y: center.y + 67)
            let row = try localFrame(of: button("Hide Slice A", in: window), in: window)
            let rest = try MountedViewPixels(capturing: window.host, for: "\(kind) at rest", in: space)
            let colour = try rest.pixel(at: CGPoint(x: row.minX + 24, y: row.midY))
            let restOther = try rest.pixel(at: other).alphaComponent
            let restPopped = try rest.pixel(at: popped).alphaComponent
            let restText = try tooltipText(in: window)
            #expect(colour.alphaComponent > 0.99)
            #expect(restOther > 0.99)
            #expect(restPopped < 0.01)
            #expect(restText.isEmpty)

            await movePlotHover(to: CGPoint(x: center.x + 61, y: center.y + 61), plot: plot, hover: hover, in: window)
            let hovered = try MountedViewPixels(capturing: window.host, for: "\(kind) with plot hover", in: space)
            let dimmed = try hovered.pixel(at: other).alphaComponent
            #expect(abs(dimmed - 0.4) < 0.02)
            if kind == "donut" {
                let track = try rest.pixel(at: otherTrack).alphaComponent
                let dimmedTrack = try hovered.pixel(at: otherTrack).alphaComponent
                #expect(track > 0.02)
                #expect(abs(dimmedTrack - track * 0.4) < 0.01)
            }
            let poppedAlpha = try hovered.pixel(at: popped).alphaComponent
            #expect(poppedAlpha > 0.99)
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
            await movePlotHover(to: popped, plot: plot, hover: hover, in: window)
            let cleared = try MountedViewPixels(capturing: window.host, for: "\(kind) after plot hover clears", in: space)
            let clearedPopped = try cleared.pixel(at: popped).alphaComponent
            let clearedOther = try cleared.pixel(at: other).alphaComponent
            let clearedText = try tooltipText(in: window)
            #expect(clearedPopped < 0.01)
            #expect(clearedOther > 0.99)
            #expect(clearedText.isEmpty)
        }

        @Test(arguments: ["pie", "donut"],
              ["Zero", "Debt"].flatMap { label in WindowColorSpace.allCases.map { (label, $0) } })
        func hoveringANonPositiveLegendRowDimsThePlot(kind: String, variant: (String, WindowColorSpace)) async throws {
            let (label, space) = variant
            let hover = PolarChartHoverInput()
            let window = try await mount("""
            root = Col([Chart("\(kind)", [{name:"Positive",value:4},{name:"Zero",value:0},{name:"Debt",value:-2}])], 0)
            """, kind: kind, hover: hover, colorSpace: space)
            defer { window.window.close() }

            let plot = try plotFrame(kind, in: window)
            let wedge = CGPoint(x: plot.midX + 85, y: plot.midY)
            let rest = try MountedViewPixels(capturing: window.host, for: "\(kind) before legend hover", in: space)
            let restAlpha = try rest.pixel(at: wedge).alphaComponent
            #expect(restAlpha > 0.99)
            let row = try localFrame(of: button("Hide " + label, in: window), in: window)
            try #require(!row.isEmpty && window.host.bounds.contains(CGPoint(x: row.midX, y: row.midY)))
            hover.target = .legend(PolarChartData.Label(label))
            await window.settle()
            let hovered = try MountedViewPixels(capturing: window.host, for: "\(kind) hovering legend row \(label)", in: space)
            let dimmed = try hovered.pixel(at: wedge).alphaComponent
            let hoveredText = try tooltipText(in: window)
            #expect(abs(dimmed - 0.4) < 0.02)
            #expect(hoveredText.isEmpty)

            await movePlotHover(to: CGPoint(x: plot.minX + 2, y: plot.minY + 2), plot: plot, hover: hover, in: window)
            let cleared = try MountedViewPixels(capturing: window.host, for: "\(kind) after legend hover clears", in: space)
            let clearedAlpha = try cleared.pixel(at: wedge).alphaComponent
            #expect(clearedAlpha > 0.99)
        }

        @Test(arguments: ["pie", "donut"])
        func plotHoverUsesTheEffectiveOuterRadius(kind: String) async throws {
            let hover = PolarChartHoverInput()
            let window = try await mount("""
            root = Col([Chart("\(kind)", [{name:"Only",value:1}])], 0)
            """, kind: kind, width: 399, height: 500, hover: hover)
            defer { window.window.close() }

            let chart = try window.rect("/children/0")
            let plot = try plotFrame(kind, in: window)
            let center = CGPoint(x: chart.midX, y: chart.minY + 150)
            await movePlotHover(to: CGPoint(x: center.x + 90, y: center.y), plot: plot, hover: hover, in: window)
            let insideText = try tooltipText(in: window)
            #expect(!insideText.isEmpty)
            await movePlotHover(to: CGPoint(x: center.x + 155, y: center.y), plot: plot, hover: hover, in: window)
            let outsideText = try tooltipText(in: window)
            #expect(outsideText.isEmpty)
        }

        // Temporary opt-in for the display CI branch. Keep false on main.
        private static let runHoverProbeByDefault = false

        @Test func polarHoverDisplayProbe() async throws {
            let enabled = ProcessInfo.processInfo.environment["OPENUI_POLAR_HOVER_PROBE"]
            guard enabled.map({ $0 == "1" }) ?? Self.runHoverProbeByDefault else { return }

            var callbacks: [(phase: HoverPhase, pointer: CGPoint)] = []
            let window = try await mount("""
            root = Col([Chart("pie", [{name:"Slice A",value:3},{name:"Slice B",value:1}])], 0)
            """, kind: "pie", onPlotHover: { phase in
                callbacks.append((phase, NSEvent.mouseLocation))
            })
            defer { window.window.close() }

            let before = probeCapture("before synthetic move", in: window)
            probeTrackingAreas(in: window.host)
            let plot = try plotFrame("pie", in: window)
            let other = CGPoint(x: plot.midX - 82, y: plot.midY - 25)
            probePixel(before, at: other)

            let point = CGPoint(x: plot.midX + 61, y: plot.midY + 61)
            let location = window.host.convert(point, to: nil)
            let move = try #require(NSEvent.mouseEvent(with: .mouseMoved, location: location,
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 0, pressure: 0), "Could not create the probe mouse move")
            let receiver = window.host
            let hit = receiver.hitTest(receiver.superview?.convert(location, from: nil) ?? location)
            probeWindow(window.window, at: "before dispatch")
            print("[polar-hover-probe] mouseMoved window pt \(location) -> \(type(of: receiver)),"
                + " hitTest \(hit.map { String(describing: type(of: $0)) } ?? "none"), count before \(callbacks.count)")
            receiver.mouseMoved(with: move)
            print("[polar-hover-probe] callback count immediately after dispatch \(callbacks.count)")
            await window.settle()
            probeWindow(window.window, at: "after settle")
            print("[polar-hover-probe] callback count after settle \(callbacks.count)")
            for (index, callback) in callbacks.enumerated() {
                print("[polar-hover-probe] callback \(index + 1): \(callback.phase), pointer screen pt \(callback.pointer)")
            }
            probePixel(probeCapture("after synthetic move", in: window), at: other)
            do {
                let text = try tooltipText(in: window)
                print("[polar-hover-probe] tooltip text \(text.map(\.value))")
            } catch {
                print("[polar-hover-probe] tooltip lookup error \(error)")
            }
        }

        private func probeCapture(_ stage: String, in window: CollectionWindow) -> MountedViewPixels? {
            let content = window.window.contentRect(forFrameRect: window.window.frame)
            print("[polar-hover-probe] \(stage): content \(content.size) pt,"
                + " window backing scale \(window.window.backingScaleFactor)")
            do {
                let capture = try MountedViewPixels(capturing: window.host, for: stage)
                print("[polar-hover-probe] capture succeeded: \(capture.description)")
                return capture
            } catch {
                print("[polar-hover-probe] capture failed: \(error)")
                return nil
            }
        }

        private func probePixel(_ capture: MountedViewPixels?, at point: CGPoint) {
            guard let capture else { return }
            do {
                print("[polar-hover-probe] pixel at \(point) pt: \(try capture.pixel(at: point))")
            } catch {
                print("[polar-hover-probe] pixel lookup failed: \(error)")
            }
        }

        private func probeTrackingAreas(in host: NSView) {
            var pending = [host]
            while let view = pending.popLast() {
                let areas = view.trackingAreas
                if view === host || !areas.isEmpty {
                    let details = areas.map { area in
                        let owner = area.owner.map { String(describing: type(of: $0)) } ?? "none"
                        return "rect \(area.rect) options \(area.options) owner \(owner)"
                    }
                    print("[polar-hover-probe] \(type(of: view)) tracking areas (\(areas.count)): \(details)")
                }
                pending.append(contentsOf: view.subviews)
            }
        }

        private func probeWindow(_ window: NSWindow, at stage: String) {
            let pointer = NSEvent.mouseLocation
            print("[polar-hover-probe] \(stage): window frame \(window.frame), key \(window.isKeyWindow),"
                + " app active \(NSApp.isActive), acceptsMouseMovedEvents \(window.acceptsMouseMovedEvents),"
                + " visible \(window.isVisible), screen \(String(describing: window.screen?.frame)),"
                + " real pointer \(pointer), in window \(window.frame.contains(pointer))")
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

        @Test(arguments: [IntelligentUIMode.light, .dark])
        func sideLegendChevronsDrawAtTwelvePointsInPrimaryColour(mode: IntelligentUIMode) async throws {
            let window = try await CollectionWindow(program(14), width: 440, height: 360, mode: mode) { element, path in
                guard path == "/children/0", let data = PolarChartData(element) else { return nil }
                // Plain buttons expose the label's own bounds without a bezel or padding.
                // Use a distinct theme ink so the primary paint comparison can tell them apart.
                return AnyView(PolarChart(kind: .pie, data: data).buttonStyle(.plain)
                    .environment(\.intelligentInk, .hex(0xff00ff))
                    .overlay(alignment: .topLeading) { Rectangle().fill(.primary).frame(width: 8, height: 8) })
            }
            defer { window.window.close() }
            window.window.appearance = NSAppearance(named: mode == .light ? .aqua : .darkAqua)
            await window.settle()

            // One step down leaves both controls enabled for the same paint comparison.
            let down = try button("Scroll legend down", in: window)
            try #require(down.accessibilityPerformPress?() == true)
            await window.settle()
            let capture = try MountedViewPixels(capturing: window.host, for: "legend chevrons in \(mode)")
            let chart = try window.rect("/children/0")
            let primary = try capture.pixel(at: CGPoint(x: chart.minX + 4, y: chart.minY + 4))
            #expect(primary.alphaComponent == 1)
            #expect(mode == .light ? primary.redComponent < 0.2 : primary.redComponent > 0.8)

            for (label, direction): (String, CGFloat) in [("Scroll legend up", -1), ("Scroll legend down", 1)] {
                let control = try button(label, in: window)
                #expect(control.isAccessibilityEnabled?() == true, "\(label) in \(mode)")
                let frame = try localFrame(of: control, in: window)
                try #require(frame.size == CGSize(width: 12, height: 12), "\(label) in \(mode)")
                // Sample inside the apex and both wings, and check their vertical reflections are clear.
                for dx: CGFloat in [-2.5, 0, 2.5] {
                    let dy = (dx == 0 ? 0.75 : -0.75) * direction
                    let stroke = try capture.pixel(at: CGPoint(x: frame.midX + dx, y: frame.midY + dy))
                    let clear = try capture.pixel(at: CGPoint(x: frame.midX + dx, y: frame.midY - dy))
                    #expect(stroke.alphaComponent > 0.3 && clear.alphaComponent < 0.05, "\(label) at x offset \(dx) in \(mode)")
                    #expect(abs(stroke.redComponent - primary.redComponent) < 0.03, "\(label) in \(mode)")
                    #expect(abs(stroke.greenComponent - primary.greenComponent) < 0.03, "\(label) in \(mode)")
                    #expect(abs(stroke.blueComponent - primary.blueComponent) < 0.03, "\(label) in \(mode)")
                }
            }
        }

        private func mount(_ program: String, kind: String, width: CGFloat = 200, height: CGFloat = 420,
                           hover: PolarChartHoverInput? = nil, colorSpace: WindowColorSpace? = nil,
                           onPlotHover: (@MainActor @Sendable (HoverPhase) -> Void)? = nil) async throws -> CollectionWindow {
            let kind = try #require(PolarChartKind(rawValue: kind))
            return try await CollectionWindow(program, width: width, height: height, colorSpace: colorSpace) { element, path in
                guard path == "/children/0", let data = PolarChartData(element) else { return nil }
                return AnyView(PolarChart(kind: kind, data: data, hoverInput: hover, onPlotHover: onPlotHover))
            }
        }

        private func movePlotHover(to point: CGPoint, plot: CGRect, hover: PolarChartHoverInput,
                                   in window: CollectionWindow) async {
            hover.target = .plot(.active(CGPoint(x: point.x - plot.minX, y: point.y - plot.minY)))
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
