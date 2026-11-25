import AppKit
@testable import OpenUIIntelligentUI
import SwiftUI
import Testing

extension CollectionDrawingTests {
    @Suite @MainActor struct RealPointerHoverProbeTests {
        @Test(arguments: ["pie", "bar", "line", "scatter"])
        func realPointerHover(kind: String) async throws {
            let probe = RunnerDisplayProbe.realPointer(kind)
            try await probe.run { try await measureRealPointerHover(kind: kind, prefix: probe.prefix) }
        }

        private func measureRealPointerHover(kind: String, prefix: String) async throws {
            let trace = HoverTrace()
            let session = RunnerDisplaySession(prefix: prefix)
            defer { session.restore() }
            let original = NSEvent.mouseLocation
            let originalScreen = try #require(NSScreen.screens.first { $0.frame.contains(original) },
                                              "Original pointer \(original) is outside the available screens")
            let restorePoint = try quartzPoint(original, on: originalScreen)
            defer {
                let result = CGWarpMouseCursorPosition(restorePoint)
                #expect(result == .success, "[real-pointer-probe] Failed to restore pointer to \(original): \(result)")
            }

            // Start outside the window so crossing into it can also exercise
            // any mouseEntered tracking the window server delivers.
            let parkingScreen = try #require(NSScreen.screens.first)
            let visible = parkingScreen.visibleFrame
            let park = CGPoint(x: visible.minX + 2, y: visible.maxY - 2)
            try warp(to: park, on: parkingScreen)
            let fixture = try await mount(kind, trace: trace)
            defer { fixture.window.close() }
            let window = fixture.window
            window.acceptsMouseMovedEvents = true
            do {
                try await session.present(fixture)
                let screen = try #require(window.screen)
                let expected = switch kind {
                case "pie": ["Probe Slice A", "3"]
                case "scatter": ["ProbeSeries X: 2 Y: 4"]
                default: ["2 ProbeSeries: 4"]
                }

                let start = NSEvent.mouseLocation
                try #require(!window.frame.contains(start), "Could not park the pointer outside the probe window")
                let restingText = accessibilityText(in: fixture.host)
                try #require(!trace.selected && !expected.allSatisfy(restingText.contains),
                             "Chart already has the expected tooltip before the target warp: \(restingText)")
                trace.callbacks = 0
                trace.activeCallbacks = 0

                let target: CGPoint
                if kind == "pie" {
                    let plot = try #require(accessibilityNodes(in: fixture.host).first {
                        $0.accessibilityLabel?() == "Pie chart"
                    }, "Could not measure the pie plot")
                    let frame = try #require(plot.accessibilityFrame?())
                    target = onBackingPixel(CGPoint(x: frame.midX + frame.width * 0.24,
                                                    y: frame.midY - frame.height * 0.24), in: window)
                } else {
                    let mark = try #require(trace.mark, "Could not measure the \(kind) mark with ChartProxy")
                    target = screenPoint(mark, in: fixture)
                }
                try #require(screen.frame.contains(target) && window.frame.contains(target),
                             "Measured target \(target) is outside the probe window or its screen")
                logDisplayProbe("\(prefix) warping from \(start) to \(target); \(session.describe(window))")
                try warp(to: target, on: screen)

                let deadline = ContinuousClock.now + .seconds(2)
                var text: [String] = []
                repeat {
                    session.drainEvents { event in
                        guard event.window === window else { return }
                        if event.type == .mouseMoved { trace.moves += 1 }
                        if event.type == .mouseEntered { trace.enters += 1 }
                    }
                    fixture.host.layoutSubtreeIfNeeded()
                    text = accessibilityText(in: fixture.host)
                    if trace.activeCallbacks > 0, trace.selected, expected.allSatisfy(text.contains) { break }
                    try await Task.sleep(for: .milliseconds(20))
                } while ContinuousClock.now < deadline

                let reached = NSEvent.mouseLocation
                let diagnostic = "\(prefix) pointer reached \(reached) (target \(target)); \(trace.summary); "
                    + "tooltip text=\(text); expected=\(expected)"
                let tolerance = max(1, 1 / window.backingScaleFactor)
                let failure: String?
                if !session.isFrontmost(window) {
                    failure = "probe window lost key or frontmost status during the hover wait"
                } else if abs(reached.x - target.x) > tolerance || abs(reached.y - target.y) > tolerance {
                    failure = "pointer did not stay at the measured target"
                } else if trace.callbacks == 0 && trace.moves == 0 && trace.enters == 0 {
                    failure = "pointer warped to \(reached) but no mouseMoved, mouseEntered or hover arrived in 2 s"
                } else if trace.activeCallbacks == 0 {
                    failure = "pointer input arrived but SwiftUI delivered no active hover phase in 2 s"
                } else if !trace.selected {
                    failure = "active hover arrived but the chart did not select the mark in 2 s"
                } else if !expected.allSatisfy(text.contains) {
                    failure = "chart selected the mark but the expected tooltip text did not appear in 2 s"
                } else {
                    failure = nil
                }
                if let failure {
                    throw CollectionInputFailure(description: "\(failure); \(diagnostic)")
                }
                logDisplayProbe("\(diagnostic); PASS")
            } catch {
                logDisplayProbe("\(prefix) FAIL: \(error); NSEvent.mouseLocation=\(NSEvent.mouseLocation); "
                    + "\(session.describe(window)); \(trace.summary)")
                dumpTrackingAreas(in: fixture.host, prefix: prefix)
                throw error
            }
        }

        private func mount(_ kind: String, trace: HoverTrace) async throws -> CollectionWindow {
            let chart = kind == "pie"
                ? #"Chart("pie", [{name:"Probe Slice A",value:3},{name:"Probe Slice B",value:1}])"#
                : """
                Chart("\(kind)", [{x:1,ProbeSeries:4},{x:2,ProbeSeries:4},{x:3,ProbeSeries:8},{x:4,ProbeSeries:0}],
                    "x", ["ProbeSeries"])
                """
            return try await CollectionWindow("root = Col([\(chart)], 0)", width: kind == "pie" ? 200 : 420,
                                              height: 420) { element, path in
                guard path == "/children/0" else { return nil }
                if kind == "pie", let data = PolarChartData(element) {
                    return AnyView(PolarChart(kind: .pie, data: data, onPlotHover: trace.record,
                                             onPlotSelection: { trace.selected = $0 }))
                }
                guard let kind = CartesianChartKind(rawValue: kind),
                      let data = CartesianChartData(element, kind: kind) else { return nil }
                let observation = CartesianHoverObservation(coordinateSpace: "window", layout: { proxy, plot in
                    // A bar's interior, a point on the flat line segment, or
                    // the centre of the displayed scatter dot, respectively.
                    let point = switch kind {
                    case .scatter: proxy.position(for: data.scatterCoordinates(x: 2, y: 4))
                    case .line: proxy.position(for: (x: 0.75, y: 4.0))
                    default: proxy.position(for: (x: 1.0, y: 2.0))
                    }
                    trace.mark = point.map { CGPoint(x: plot.minX + $0.x, y: plot.minY + $0.y) }
                }, phase: trace.record, selection: { trace.selected = $0 })
                return AnyView(CartesianPlot(data: data, hoverObservation: observation))
            }
        }

        private func screenPoint(_ point: CGPoint, in fixture: CollectionWindow) -> CGPoint {
            let host = fixture.host
            let local = CGPoint(x: host.bounds.minX + point.x,
                                y: host.isFlipped ? host.bounds.minY + point.y : host.bounds.maxY - point.y)
            let window = fixture.window
            return onBackingPixel(window.convertPoint(toScreen: host.convert(local, to: nil)), in: window)
        }

        private func onBackingPixel(_ point: CGPoint, in window: NSWindow) -> CGPoint {
            let backing = window.convertPointToBacking(window.convertPoint(fromScreen: point))
            let pixel = CGPoint(x: backing.x.rounded(), y: backing.y.rounded())
            return window.convertPoint(toScreen: window.convertPointFromBacking(pixel))
        }

        private func quartzPoint(_ point: CGPoint, on screen: NSScreen) throws -> CGPoint {
            let id = try #require(screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)
            let display = CGDisplayBounds(id.uint32Value)
            let frame = screen.frame
            try #require(!display.isEmpty && !frame.isEmpty, "Display has no usable bounds")
            // AppKit is bottom-left; Quartz's global display space is top-left.
            // The backing-pixel snap above does not turn display points into pixels.
            return CGPoint(x: display.minX + (point.x - frame.minX) * display.width / frame.width,
                           y: display.minY + (frame.maxY - point.y) * display.height / frame.height)
        }

        private func warp(to point: CGPoint, on screen: NSScreen) throws {
            let quartz = try quartzPoint(point, on: screen)
            let result = CGWarpMouseCursorPosition(quartz)
            guard result == .success else {
                throw CollectionInputFailure(description: "CGWarpMouseCursorPosition failed for \(point) (Quartz \(quartz)): "
                    + "\(result); NSEvent.mouseLocation=\(NSEvent.mouseLocation)")
            }
        }

        private func accessibilityNodes(in host: NSView) -> [AnyObject] {
            var pending: [AnyObject] = [host]
            var nodes: [AnyObject] = []
            while let node = pending.popLast() {
                nodes.append(node)
                pending.append(contentsOf: (node.accessibilityChildren?() ?? []).map { $0 as AnyObject })
            }
            return nodes
        }

        private func accessibilityText(in host: NSView) -> [String] {
            accessibilityNodes(in: host).compactMap { node in
                let role = node.accessibilityRole?()
                guard role == .staticText || role?.rawValue == "AXHeading" else { return nil }
                let value: String? = node.accessibilityValue?()
                let text = value ?? node.accessibilityLabel?()
                return text?.split(whereSeparator: \.isWhitespace).joined(separator: " ")
            }
        }

        private func dumpTrackingAreas(in host: NSView, prefix: String) {
            var pending = [host]
            while let view = pending.popLast() {
                let areas = view.trackingAreas
                if view === host || !areas.isEmpty {
                    let details = areas.map { area in
                        let owner = area.owner.map { String(describing: type(of: $0)) } ?? "none"
                        return "rect=\(area.rect) options=\(area.options.rawValue) owner=\(owner)"
                    }
                    logDisplayProbe("\(prefix) \(type(of: view)) tracking areas (\(areas.count)): \(details)")
                }
                pending.append(contentsOf: view.subviews)
            }
        }
    }
}

@MainActor private final class HoverTrace {
    var callbacks = 0
    var activeCallbacks = 0
    var moves = 0
    var enters = 0
    var selected = false
    var mark: CGPoint?

    func record(_ phase: HoverPhase) {
        callbacks += 1
        if case .active = phase { activeCallbacks += 1 }
    }

    var summary: String {
        "mouseMoved=\(moves) mouseEntered=\(enters) callbacks=\(callbacks) active=\(activeCallbacks) selected=\(selected)"
    }
}
